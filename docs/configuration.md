# Configuration

This page lists the files you write to set Cox up, and what each one
holds. Cox reads `settings.json`, `agents.json` and the workflow's files
again when they change, so you can edit them while Cox runs.

- [Command line](#command-line)
- [The data folder](#the-data-folder)
- [`settings.json`](#settingsjson)
- [`agents.json`](#agentsjson)
- [The workflow folder](#the-workflow-folder)
- [Triggers](#triggers)
- [`sketch.json`](#sketchjson)
- [A project's own files](#a-projects-own-files)
- [Environment variables](#environment-variables)

## Command line

```sh
cox [--stage <url>] [--home <dir>] [--data <dir>] [<agent command>]
```

| Option | Meaning |
| --- | --- |
| `<agent command>` | The agent a new chat starts, such as `npx -y @agentclientprotocol/claude-agent-acp`. Without it, Cox starts the agent `settings.json` names. |
| `--stage <url>` | The page a new stage shows first, instead of "Activity". |
| `--home <dir>` | Cox's home for this run, until you set the home in the settings. |
| `--data <dir>` | Keep everything in `<dir>` instead of the usual [data folder](#the-data-folder), to run apart from what Cox kept before, or on a copy of it. |

## The data folder

Cox keeps what lasts across runs in a data folder:

| Cox | Data folder |
| --- | --- |
| Installed by `cargo xtask install`, macOS | `~/Library/Application Support/Cox` |
| Installed by `cargo xtask install`, Linux | `~/.local/share/cox` |
| Any other build, such as `target/debug/Cox.app` | `cox-data` beside it, such as `target/debug/cox-data` |
| Started with `--data <dir>` | `<dir>` |

You write `settings.json` and `agents.json` at its top. The rest is
Cox's; [`storage.rs`](../crates/cox/src/storage.rs) lists all of it.

## `settings.json`

Every key is optional. Paths may be absolute or start with `~`.

```json
{
  "agent": "Claude",
  "home": "~/cox",
  "workflow": "~/dotfiles/cox",
  "wiki": "~/notes/cox",
  "raw": "~/notes/raw",
  "translation": {
    "language": "Japanese",
    "command": ["plamo-translate", "--from", "{from}", "--to", "{to}"]
  },
  "phone": {"port": 7421, "tailscale": true}
}
```

| Key | Meaning | Default |
| --- | --- | --- |
| `agent` | The name, in `agents.json`, of the agent a new chat starts when Cox has no `<agent command>`. | none |
| `home` | Cox's home: where Cox's own chats work, and where `projects`, `workflow`, `wiki` and `raw` are by default. | `~/cox` |
| `workflow` | Your [workflow folder](#the-workflow-folder). | `workflow` in the home |
| `wiki` | Your wiki: Markdown pages the agents write, such as an Obsidian vault or a Git repository. | `wiki` in the home |
| `raw` | Where agents keep the sources they read (`raw_keep`) in Cox's own chats, or when told to keep them for every project. A project's chats keep theirs in the project's `raw/`. | `raw` in the home |
| `translation.language` | Your language. Text in English is translated into it, and other text into English. Without it, translation is off. | none |
| `translation.command` | A command that reads text on standard input and writes its translation on standard output. `{from}` and `{to}` in its arguments stand for the languages. | none |
| `translation.agent` | The name of an agent to translate with, used when there is no `command`. | the agent a new chat starts |
| `phone` | `false` turns off the page for the phone. | on |
| `phone.port` | The port of the page for the phone. The sketches' mocks are served on the next port. | `7421` |
| `phone.tailscale` | `false` stops `coxd` from running `tailscale serve` for the page. | `true` |

The settings card (Cmd-, or Ctrl-,) sets `agent`, `home`, `workflow`,
`wiki` and `translation` for you, and leaves the rest of the file as it
is. `coxd` reads `phone` only when it starts.

## `agents.json`

A list of the agents you use, each with a name and a command. `NAME=value`
before the program sets its environment, as on a shell's command line.

```json
[
  {"name": "Claude", "command": ["CLAUDE_CODE_ENABLE_TODO_TOOLS=1", "npx", "-y", "@agentclientprotocol/claude-agent-acp"]},
  {"name": "Codex", "command": ["npx", "-y", "@agentclientprotocol/codex-acp"]}
]
```

A chat can also switch to an agent of the
[ACP Registry](https://agentclientprotocol.com/get-started/registry)
without it being listed here. `COX_REGISTRY` names another registry to
use, such as a mirror.

## The workflow folder

The workflow folder holds how you work with Cox, whatever the project.
Every file is optional; where one is missing, Cox uses its own, which is
a good starting point to copy.

| File | What it is | Cox's own |
| --- | --- | --- |
| `COX.md` | What every chat's agent is told first, before the project's `COX.md`. | [`cox.md`](../crates/cox/src/agent/instructions/cox.md) |
| `sketch/start.md` | The first line of a sketch opened beside a chat. `{folder}`, `{from}` and `{what}` stand for the sketch's folder, what it starts from and what it tries. | [`sketch/start.md`](../crates/cox/src/workflow/sketch/start.md) |
| `sketch/opened.md` | What an agent that opened a sketch with `open_chat` is told about it. | [`sketch/opened.md`](../crates/cox/src/workflow/sketch/opened.md) |
| `sketch/discard.md` | What a sketch's agent is asked before the sketch ends. `{left_out}` stands for the files too big to keep. | [`sketch/discard.md`](../crates/cox/src/workflow/sketch/discard.md) |
| `sketch.json` | How sketches end. See [`sketch.json`](#sketchjson). | [`sketch.json`](../crates/cox/src/workflow/sketch.json) |
| `wiki/save.md` | What "Save to wiki" asks the agent. | [`wiki/save.md`](../crates/cox/src/workflow/wiki/save.md) |
| `close/merge.md` | What "Merge" asks the agent. | [`close/merge.md`](../crates/cox/src/workflow/close/merge.md) |
| `icon/generate.md` | What "Generate", under the icon on a project's card, asks an agent. `{name}` stands for the project's name. | [`icon/generate.md`](../crates/cox/src/workflow/icon/generate.md) |
| `triggers/*.md` | Triggers that run in every project. See [Triggers](#triggers). | none |

## Triggers

A trigger is a Markdown file that runs on an event in a chat: in
`triggers` in your workflow folder, for every project, or in
`.cox/triggers` in a project's folder, for that project. Its name is the
file's name. The front matter says when it runs and who runs it; the
rest is what is done.

```markdown
---
on: turn.end
when:
  changed: ["**/*.rs", "!target/**"]
  sketch: skip
by: new
allow: [read:*, search:*, execute:git diff*]
---

Read this turn's diff. If anything could be simpler, say where and how.
Change nothing.
```

### `on`

The event, or a list of events:

| Event | When it runs |
| --- | --- |
| `turn.end` | A turn ends. If it fails, the chat's agent is told what it found, as the next turn. |
| `goal.met` | The chat's goal is met. |
| `wiki.written` | The agent writes a wiki page. `wiki.written: {page: "research/*"}` limits it to the pages the globs match. |
| `sketch.end` | A sketch ends, in the chat that goes on with it. |
| `chat.open` | A chat opens, once its agent has started. |
| `plan.changed` | The agent changes the plan you left it to. |
| `permission.asked` | The agent asks your permission. The trigger gathers what you need to answer; it decides nothing. |
| `schedule` | At the times `cron` says. See [Schedules](#schedules). |

Triggers of events other than `turn.end` and `permission.asked` run once
the chat waits for you, and what they find stays in the conversation;
the agent is not told.

### `when`

| Key | Meaning |
| --- | --- |
| `changed` | Globs of the chat's folder; a glob starting with `!` leaves files out. The trigger runs only when a matching file changed since it last ran in the chat, or else since the last commit. |
| `sketch` | `skip` not to run in a sketch, `only` to run only in one. |

### `by`

| Value | Who runs it |
| --- | --- |
| `shell` | The rest of the file runs as a command in a terminal on the stage. It passes when it exits 0. It can show a picture or video by printing `![caption](path)` on a line of its own. |
| `this` | The chat's own agent is told the rest as its next turn, once for each line of yours. Does not run on `permission.asked`. |
| `new` | An agent of its own, with the same command as the chat's, in the chat's folder, with none of Cox's tools. It ends its reply with `PASS`, or `FAIL: ` and what the chat has to act on. It keeps one session while the file stays the same. |

### `allow` and `reject`

The permission requests of a `new` agent that Cox answers for you. A
request both match is rejected; the rest are asked of you. A pattern
matches the call's title, as the conversation shows it, with `*` for any
run of characters, such as `Edit src/main.rs`. It can start with one of
ACP's kinds of tool call and a `:`, to match that kind whatever the agent
names its tools: `read`, `edit`, `delete`, `move`, `search`, `execute`,
`think`, `fetch`, `switch_mode` or `other`. For example, `edit:*`
matches every edit, and `execute:cargo test*` a command that starts with
`cargo test`.

Without them, a `new` agent may do what a goal's judge may: read and
search, and run the usual tests, such as `execute:git diff*` and
`execute:cargo test*`.

### Schedules

A trigger `on: schedule` opens a new chat at the times it says, whether
a window is open or not, and sends the rest of the file as its first
line, as you.

```markdown
---
on:
  schedule:
    cron: "0 9 * * *"
by: new
project: notes
allow: [fetch:*, search:*]
---

Find out what changed in Rust since {last}. Keep each source you read
with raw_keep, under the topic `{name}`, then write what changed, with
the sources it comes from, to a page of the wiki, `research/{name}`.
```

| Key | Meaning |
| --- | --- |
| `cron` | The times it runs, in local time, as cron writes them: minute, hour, day of month, month and day of week. `0 9 * * 1-5` is nine on weekdays; `0 */6 * * *` is every six hours. |
| `by` | Always `new`. |
| `project` | The project the chat works in, by name, in `projects` in the home; Cox makes its folder if needed. Without it, the chat works in the home, or, for a project's trigger, in the project. |
| `allow`, `reject` | As above. |

In the first line, `{name}` stands for the trigger's name, `{now}` for
when it runs, and `{last}` for when it last ran, or `never`. A schedule
that missed runs, as while the machine slept, runs once; one whose last
chat is still at work waits.

## `sketch.json`

How sketches end, in your workflow folder. `allow` lists the calls a
sketch's agent makes without asking, as [`allow`](#allow-and-reject)
takes them. `endings` lists the ways a sketch ends, in the order the
menu offers them.

```json
{
  "allow": ["edit:*", "delete:*", "move:*"],
  "endings": [
    {"name": "Rewrite in project", "from": "project",
     "sum_up": "sketch/discard.md", "keep": "ref",
     "then": "sketch/rewrite.md", "goal": "$ cargo xtask ci"},
    {"name": "Discard sketch", "sum_up": "sketch/discard.md", "keep": "ref"}
  ]
}
```

| Key | Meaning |
| --- | --- |
| `name` | The name the menu shows. |
| `from` | `project` or `empty`, to offer the ending only for sketches that start from the project or from nothing. |
| `sum_up` | A file of the workflow the sketch's agent is asked first, to sum the sketch up, with `{left_out}`. |
| `keep` | `"ref"` keeps what the sketch changed, as one commit: under `refs/cox/discarded/` in the project's repository, or as a Git bundle in `sketches/discarded` in the data folder for a sketch from nothing. |
| `folder` | `"open"` makes a new project for what a sketch from nothing tried. What the sketch made is kept apart, as a discarded sketch's is, and the project starts without it. |
| `then` | A file of the workflow that the chat the sketch was opened beside is told next. `{kept}` stands for the ref or bundle, `{summary}` for the sketch's summing up, and `{source}` for the source Cox keeps of the sketch. |
| `goal` | The goal that chat gets: `$ ` and a command, or words. |

However a sketch ends, and whatever `sketch.json` says, Cox keeps what it
did as a source of the project's, under the topic `sketches` in its
`raw/`: how it ended, all that was said in it, and the pictures and
recordings it showed, copied into `sketches/media/`. A sketch of Cox's
own chats keeps it in the human's `raw`. A sketch's code never becomes
the project's: a project made for a sketch starts without it, to make
what the sketch tried again.

## A project's own files

| File | What it is |
| --- | --- |
| `COX.md` | What the project's chats' agents are told first, after the workflow's `COX.md`. |
| `.cox/goal.md` | What a goal's judge is told it is, instead of Cox's default. Cox still adds how to end the reply, and the goal. |
| `.cox/triggers/*.md` | The project's [triggers](#triggers). |
| `wiki/` | The project's own wiki, for what holds only of it. Only its chats see it. |
| `raw/` | The project's own sources, what its chats read (`raw_keep`), as they read it. Only its chats see them. |
| `.cox/maps/<name>.json` | The project's maps. Agents write them; `show_map` tells an agent the format. |
| `.cox/plugins/<name>/` | A [plugin](#plugins): a view of the project's own on the stage. |
| `.cox/servers.json` | Language servers for the project. See [Language servers](#language-servers). |

### Language servers

Cox starts a language server for a code tab's language, if one is on
`PATH`: `rust-analyzer`, `pyright-langserver`,
`typescript-language-server`, `gopls` or `clangd`. A project names
servers of its own in `.cox/servers.json`, in the same form as Cox's
[`servers.json`](../crates/cox-editor/src/lsp/servers.json):

```json
{
  "servers": {
    "rust-analyzer": {
      "settings": {"rust-analyzer": {"cargo": {"features": "all"}}}
    },
    "gopls": null,
    "zls": {
      "command": ["zls"],
      "languages": ["zig"],
      "markers": ["build.zig"]
    }
  }
}
```

Each server, by name, has:

| Key | Meaning |
| --- | --- |
| `command` | The command that starts the server. |
| `languages` | The languages it serves, as LSP names them (`rust`, `python`, `typescriptreact`, ...). |
| `markers` | Files a folder of such a project has. |
| `initializationOptions` | Sent to the server as it starts. |
| `settings` | Sent by section, as the server asks for them. |

A project's server for a language goes before Cox's. For a server Cox
already knows, each key the project gives replaces Cox's, as for
`rust-analyzer` above; `null` turns the server off, as for `gopls`
above.

### Plugins

A plugin is a folder with an `index.html`, and optionally a
`plugin.json` naming its `title`. The stage's bar shows a button for it,
which opens it at `cox://<name>/`. The page reads what Cox knows as JSON
from `cox://api/...`, and draws in Cox's colors by linking
`cox://api/theme.css`. Cox's own plugins, in
[`crates/cox/plugins`](../crates/cox/plugins), show how;
[`plugins.rs`](../crates/cox/src/stage/plugins.rs) lists what
`cox://api/` answers.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `RUST_LOG` | What Cox writes to standard error. Errors only by default. |
| `COX_LOG` | What Cox keeps in its log files in `logs` in the data folder. Down to `debug` by default. |
| `COX_REGISTRY` | Another ACP Registry to fetch, such as a mirror. |
| `ANDROID_HOME`, `ANDROID_SDK_ROOT` | The Android SDK whose emulator Cox runs. |
| `ANDROID_AVD_HOME`, `ANDROID_EMULATOR_HOME`, `ANDROID_USER_HOME` | Where Cox finds the Android virtual devices. |
