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

You write `settings.json`, `agents.json` and `servers.json` at its top,
or change them on the settings screen. The rest is Cox's; [`storage.rs`](../crates/cox/src/storage.rs) lists all of it.

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
  "phone": {"port": 7421, "tailscale": true},
  "vim": true
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
| `phone.port` | The port of the page for the phone. The relay, which passes the pages served on the Mac's own address on to the phone, listens on the next port, and the sketches' mocks are served on the one after. | `7421` |
| `phone.tailscale` | `false` stops `coxd` from running `tailscale serve` for the page and the pages the relay passes on. | `true` |
| `vim` | `true` makes the keys work as in Vim. See [Vim's keys](#vims-keys). | `false` |

The settings card (Cmd-, or Ctrl-,) sets `agent`, `home`, `workflow`,
`wiki` and `translation` for you, and leaves the rest of the file as it
is. `coxd` reads `phone` only when it starts, and the window `vim`.

### Vim's keys

With `"vim": true`, the chat's input has a mode, shown beside it as
`NORMAL` or `INSERT`. A new chat starts in insert mode, where keys type
as usual.

| Mode | Key | Does |
| --- | --- | --- |
| Insert | Esc | Goes to normal mode, and closes the list of commands or files over the input. It does not stop the agent. |
| Normal | `i`, `a`, `I`, `A` | Goes to insert mode at the cursor, after it, at the start of the line, or at its end. |
| Normal | Ctrl-C | Stops the agent. |
| Normal | Esc | Forgets a count typed. In the goal's, a sketch's or a rename's input, the settings or a project's card, it closes them as Esc does without Vim. |
| Normal | Ctrl-W `h`, Ctrl-W `l` | Moves the keyboard to the part of the window at the left or the right: the rail, the stage, the chat. |
| Normal | Ctrl-W `w` | Moves the keyboard to the next part, round from the chat to the rail. |
| Normal | Ctrl-W `k` | In the chat, opens the list of the project's chats, with the keyboard in it. Ctrl-W `j` or Esc closes it. |
| Normal | `gt`, `gT` | Shows the stage's next tab, or the one before. `3gt` shows its third tab, `3gT` the tab three before. |
| Normal | Ctrl-W `c` | Closes the tab the stage shows. |

On the rail and in the list of chats, `j` and `k` move the selection,
`3j` three rows, `gg` to the first and `G` to the last. Enter or `l`
opens what is selected: a project's chat, or the chat, with the keyboard
in its input in normal mode. `o` opens a new chat in the project
selected. The keyboard in the stage shows as a ring round its tab, and
on the rail and the list as a ring round the row selected.

In normal mode, keys that would type type nothing, and digits make a
count, so they choose an option the agent asks only in insert mode. Keys with Cmd work as without Vim. The inputs that set a goal,
start a sketch or rename a chat open in insert mode, and the settings
and a project's card in normal mode, and so do the rail, the list of
chats and the stage's tabs. A terminal, an Android device and a web page
keep every key of their own, Ctrl-W too, which a shell needs to delete a
word: from one of them, the mouse or the app's own keys, such as
Ctrl-Tab to the next chat, take the keyboard elsewhere. While an input
method composes, every key is its own.

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
`triggers` in a project's folder, for that project. Its name is the
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

A pattern matches a command's words, as the shell hands them to it, one
space between, and allows only a command the shell runs alone: none
with `;`, `&`, `|`, `<`, `>`, `(`, `)`, `$`, a backtick, `*`, `?`, `[`,
`{`, `}` or `#` out of quotes, a `$` or backtick in double quotes, a `^`
that starts a word, or a new line, and none with a flag that has it run
another program or write a file, such as `find`'s `-exec` and
`-delete`, `rg`'s `--pre` or `git`'s `--output`. A command that checks
or tests the project, `cargo test`, `cargo check`, `cargo clippy`,
`pytest` or `go test`, is allowed only with what picks what it checks
and how much it says: packages, targets, features, names of tests,
paths in the folder, `-k`, `-run` and the like, and after
`cargo clippy --` the lints to deny, warn of, allow or forbid.

Without `allow` and `reject`, a `new` agent may do what a goal's judge
may: read and search, and run the usual tests, such as `git diff` and
`cargo test`, and `npm test`, `make test` and `cargo xtask ci` only as
they are, with nothing after them.

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

A project keeps these in its folder, `projects/<name>` in Cox's home,
beside its `directories/`. An earlier Cox read all but `COX.md`,
`wiki/` and `raw/` from `.cox/` in the folder; it reads them there no
more, so move them up out of it.

| File | What it is |
| --- | --- |
| `COX.md` | What the project's chats' agents are told first, after the workflow's `COX.md`. |
| `goal.md` | What a goal's judge is told it is, instead of Cox's default. Cox still adds how to end the reply, and the goal. |
| `triggers/*.md` | The project's [triggers](#triggers). |
| `wiki/` | The project's own wiki, for what holds only of it. Only its chats see it. |
| `raw/` | The project's own sources, what its chats read (`raw_keep`), as they read it. Only its chats see them. |
| `maps/<name>.json` | The project's maps. Agents write them; `show_map` tells an agent the format. |
| `plugins/<name>/` | A [plugin](#plugins): a view of the project's own on the stage. |
| `servers.json` | Language servers for the project. See [Language servers](#language-servers). |

### Language servers

Cox starts a language server for a code tab's language:
`rust-analyzer`, which Cox downloads the first time it needs it,
`gopls`, which Cox installs with `go install` the first time it needs it
(so `go` has to be on `PATH`), or `pyright-langserver`,
`typescript-language-server` or `clangd`, if it is on `PATH`. You name
servers of your own, for every project, in `servers.json` in the
[data folder](#the-data-folder), as the settings screen's "Language
servers" does, and a project names its own in `servers.json` in its
folder, both in the same form as Cox's
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
| `source` | Where Cox gets the server's program, instead of finding it on `PATH`. Not in a project's `servers.json`. |
| `version` | The version of the program Cox gets from the `source`. |

A project's server for a language goes before yours, and yours before
Cox's. For a server Cox already knows, each key the file gives replaces
Cox's, as for `rust-analyzer` above; `null` turns the server off, as for
`gopls` above.

#### Where Cox gets a server

Where a server's program comes from, its `source`, is Cox's to say, and
yours, in your `servers.json`, along with the version you use by
default. A project says only which `version` it uses:

```json
{"servers": {"rust-analyzer": {"version": "2026-10-01"}}}
```

A `source` is one of:

| Source | Meaning |
| --- | --- |
| `{"github": "<owner>/<repo>", "asset": "...", "program": "..."}` | The `asset` of the GitHub release tagged with the version, and the `program`'s path in it once unpacked. Cox checks the asset by the SHA-256 GitHub gives of it, and does not download an asset GitHub gives none of. |
| `{"go": "<package>"}` | The Go package, installed with `go install <package>@<version>`, which checks it by Go's checksum database. |

In `asset` and `program`, `{version}` stands for the version, `{target}`
for the platform as Rust names it (`aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu`, ...), and `{os}` and `{arch}` for it as Cox
does (`darwin`, `linux`; `aarch64`, `x86_64`). Cox's own `rust-analyzer`
is
`{"github": "rust-lang/rust-analyzer", "asset": "rust-analyzer-{target}.gz", "program": "rust-analyzer-{target}"}`.

Cox keeps each version apart, in `cache/servers/<name>/<version>` in
its data folder, so projects that use different versions of a server
each run their own. A file that gives one of Cox's servers a `command`
and no `source` runs that command instead, as `["rust-analyzer"]` to
run the one on `PATH`.

The environment says a server's version and source again, over the
files: `COX_SERVER_<NAME>_VERSION` and `COX_SERVER_<NAME>_SOURCE`, the
source as JSON, `<NAME>` being the server's name upper-cased with `_`
for what is not a letter or a digit, as
`COX_SERVER_RUST_ANALYZER_VERSION`. Cox reads its environment as it
starts, so these hold for every project.

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
| `COX_SERVER_<NAME>_VERSION`, `COX_SERVER_<NAME>_SOURCE` | The version, and the source, of the language server `<name>` that Cox gets. See [Where Cox gets a server](#where-cox-gets-a-server). |
| `ANDROID_HOME`, `ANDROID_SDK_ROOT` | The Android SDK whose emulator Cox runs. |
| `ANDROID_AVD_HOME`, `ANDROID_EMULATOR_HOME`, `ANDROID_USER_HOME` | Where Cox finds the Android virtual devices. |
