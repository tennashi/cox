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
or change them on the settings screen. The rest is Cox's; [`storage.rs`](../crates/coxd/src/storage.rs) lists all of it.

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
  "telemetry": {"endpoint": "http://127.0.0.1:4318"},
  "vim": true,
  "search": "https://duckduckgo.com/?q=%s",
  "subagents": "chats"
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
| `phone` | `false` turns off the page for the phone, and `true` turns it on for a Cox that is not installed, such as one `cargo xtask run` runs. | on for an installed Cox, off for any other |
| `phone.port` | The port of the page for the phone. The relay, which passes the pages served on the Mac's own address on to the phone, listens on the next port, and the sketches' mocks are served on the one after. | `7421` |
| `phone.tailscale` | `false` stops `coxd` from running `tailscale serve` for the page and the pages the relay passes on, and `true` has a Cox that is not installed run it. | `true` for an installed Cox, `false` for any other |
| `telemetry.endpoint` | The OTLP endpoint `coxd` sends what it measures of itself to, from `http://` or `https://`. See [Telemetry](#telemetry). | none: nothing is sent |
| `vim` | `true` makes the keys work as in Vim. See [Vim's keys](#vims-keys). | `false` |
| `search` | The search engine: the address of its results, from `http://` or `https://`, with `%s` where the words go. Cox searches with it for the words typed in a page's address line, as a browser does (a `?` first searches for whatever follows), in the menu of the stage's +, and after Vim's `:open` or `:tabopen`. One without `%s` is not used. | `https://www.google.com/search?q=%s` |
| `subagents` | Where a chat's agent runs the subagents it hands work to. `"chats"` runs each in a chat of its own, under the chat that runs it, which you see and can step into: Cox starts Claude Code's sessions with its own `Agent` and `Task` tools turned off, so that it, and a skill that spawns subagents, uses Cox's `agent` tool in their place, with the same arguments. Its `agent` says so first, and Claude Code loads it always, rather than only once the agent searches for it, as it does Cox's other tools, so that an agent that reaches for `Agent` or `Task` finds it. Other agents read no such option and keep their own. `"agent"` leaves the subagents to the agent, out of sight. A chat keeps what the setting said when its session started, so a change applies to chats started after it, and to a chat whose agent starts a new session, as when you switch its agent. | `"agent"` |

The settings card (Cmd-, or Ctrl-,) sets `agent`, `home`, `workflow`,
`wiki`, `translation`, `vim` and `search` for you, and leaves the rest of the file
as it is. `coxd` reads `phone` and `telemetry` only when it starts.

### Telemetry

`coxd` can send how long its work for the phone's page takes as
OpenTelemetry metrics, so that you can watch the page's speed in a tool
such as Grafana. Nothing is sent unless you turn it on: without
`telemetry.endpoint`, `coxd` measures nothing, opens no connection, and
the phone's page reports nothing.

To turn it on:

1. Run something that takes OTLP over HTTP on the Mac, such as an
   OpenTelemetry Collector, or Grafana's `grafana/otel-lgtm` image.
2. Name its endpoint in `settings.json`, without `/v1/metrics`:

   ```json
   {"telemetry": {"endpoint": "http://127.0.0.1:4318"}}
   ```

`coxd` reads the setting when it starts, as it does `phone`, so it
takes effect the next time `coxd` starts.

`coxd` then posts its metrics to `<endpoint>/v1/metrics` once a minute,
as the service `coxd`. Set `OTEL_METRIC_EXPORT_INTERVAL`, in
milliseconds, to send more often. The phone's page posts what it
measures of itself to `coxd` every few seconds, and `coxd` sends it with
its own.

Every instrument is a histogram. A label takes only the values listed
here, and `other` for anything else. No label holds a chat's number, a
file's name or anything typed.

| Instrument | Unit | Labels | Measures |
| --- | --- | --- | --- |
| `cox.phone.http.duration` | s | `route`, `status` | How long `coxd` took to have its answer to a request of the phone. For the stream of events, until the stream opened. |
| `cox.phone.http.response.size` | By | `route`, `status` | The size of an answer to the phone, as sent. |
| `cox.phone.ask.wait` | s | `ask` | How long a request of the phone waited for `coxd`'s main loop. |
| `cox.phone.sse.event.size` | By | `kind` | The size of an event told to the phone. |
| `cox.phone.sse.encode.duration` | s | `kind` | How long `coxd` took to write an event for the phone as JSON. |
| `cox.phone.sse.queue` | 1 | none | How many events waited behind the one `coxd` told the phone. |
| `cox.daemon.hear.duration` | s | `event` | How long `coxd`'s main loop took over one thing it heard. |
| `cox.daemon.settle.duration` | s | none | How long the main loop took to settle after it, as listing the chats again. |
| `cox.daemon.loop.lag` | s | none | How late `coxd`'s one thread came back to a timer. |
| `cox.daemon.chat.read.duration` | s | none | How long `coxd` took to read a closed chat as it was opened. |
| `cox.daemon.chat.read.entries` | 1 | none | How many entries that chat held. |
| `cox.phone.page.cold_start` | s | `client`, `phase` | How long after the page was asked for a cold start reached each phase: `response`, `dom`, `fcp`, `listing`, `drawn`. |
| `cox.phone.page.resource.duration` | s | `client`, `file` | How long the browser took to fetch a file of the page. |
| `cox.phone.page.chat_open` | s | `client`, `phase` | How long after the tap the opening of a chat reached each phase: `listing`, `patch`, `drawn`. |
| `cox.phone.page.draw.duration` | s | `client`, `view` | How long the page's script took over one draw of `home` or `chat`. |
| `cox.phone.page.draw.to_paint` | s | `client`, `view` | How long from the start of a draw until the browser had painted it. |
| `cox.phone.page.sse.parse.duration` | s | `client`, `kind` | How long the page took to parse an event. |
| `cox.phone.page.lag` | s | `client` | How late a timer of the page's thread fired: the most in each report. |

`route` is the pattern of the path asked for, such as `/events` or
`/media/*`. `client` is `ios`, `android` or `other`, read from the
browser's name. [`instruments.rs`](../crates/coxd/src/telemetry/instruments.rs)
lists every label's values.

### Vim's keys

With `"vim": true`, the keys work as in Vim, and in a web page as in
Vimperator. Off, nothing changes. The settings card turns them on and
off under **Vim's keys**, and so does editing the file. Either takes
effect at once.

**Modes.** The keys are in one of five modes, Normal, Insert, Visual, Hint
or Command, shown in a pill at the left of the window's foot, as Vim's
`showmode`, wherever the keyboard is. At its right, as Vim's `showcmd`, are the count and the keys typed
toward a command that is not yet whole, as `3g` of `3gt`. The line `:`
and `/` open takes its place. In normal mode keys do things; keys that
would type type nothing, and digits make a count, as `3` of `3j`. In
insert mode keys type. Hint mode picks what to click by its label, and
command mode types in the line at the window's foot. Keys with Cmd work as without Vim. While an input
method composes, every key is its own.

**Regions.** Where the keyboard is decides what the keys do: the rail
of projects, the list of chats, the chat, and each tab of the stage.
The keyboard comes into a region in its own mode:

| Region | Comes in |
| --- | --- |
| The chat's input | As it was: a new chat in insert mode. |
| The inputs of a goal, a sketch, a rename | Insert mode. |
| A page's address line | Typed in always, as a field of the page is: it has no normal mode, and Esc puts the page's address back and gives the keyboard back to the page. |
| The rail, the list of chats, the chat's conversation, the settings, a project's card, code, a document, the tree, changes | Normal mode. |
| An input of the settings or of a project's card, gone to by a click, a hint, or a button that opens it, such as a directory's Link… | Insert mode, as a web page's field. |
| A web page | Insert mode while a field of it has the keyboard, else normal mode. |
| A terminal, an Android device | Normal mode, as Neovim's terminal: `i` or a click goes to insert mode, where every key goes to the program. A terminal just opened from the stage's + is in insert mode. Back from the line at the window's foot, the mode the line left. |

**Everywhere in normal mode**, but in the inputs and screens that open
over the chat:

| Key | Does |
| --- | --- |
| Ctrl-W `h`, Ctrl-W `l` | Moves the keyboard to the part at the left or the right: the rail, the tree of files when it is beside a file, the stage, the chat. The chat's conversation is part of the chat. |
| Ctrl-W `w` | Moves the keyboard to the next part, round from the chat to the rail. |
| `gt`, `gT` | Shows the stage's next tab, or the one before, among those listed beside the stage: the views on its bar, as the map or Knowledge, are left out. `3gt` shows its third tab, `3gT` the tab three before. |
| Ctrl-W `c` | Closes the tab the stage shows. |
| `:` | Opens the line at the window's foot for a command. |
| `f` | Draws hints over what can be clicked or typed in, in the part with the keyboard: the rail, the list of chats, the chat, the settings, a project's card, or the stage, its tabs' bar and the tab shown. A page draws its own. With nothing there to click, as when the chat is folded away with the keyboard in it, over the whole window, as `F`. |
| `F` | Draws hints over the whole window, the page shown's among them, in one run of labels. |

Typing a label narrows the hints to those it starts; the one left is
clicked, or, if it is an input, gets the keyboard in insert mode. Esc
takes the hints away. What is scrolled out of sight, or under the
settings, a project's card or a menu, has no hint.

A lighter edge shows which part has the keyboard: the rail, the stage
or the chat. On the rail and in the list of chats, a ring marks the row
selected.

**The chat.** Its input's keys work the same in the goal's, a sketch's and a rename's input, and in those of the settings and a project's card.

| Mode | Key | Does |
| --- | --- | --- |
| Insert | Esc | Goes to normal mode, the cursor on the character before it, as Vim's, and closes the list of commands or files over the input. It does not stop the agent. |
| Insert | Ctrl-W, Ctrl-U | Takes out the word before the cursor, or the line before it, as in Vim and a shell. In any input. |
| Normal | `i`, `a`, `I`, `A` | Goes to insert mode at the cursor, after it, at the start of the line, or at its end. |
| Normal | `h`, `l`, `w`, `b`, `e`, `0`, `^`, `$` | Moves the input's cursor, which is on a character, by characters and words, and to the line's start, first character and end. |
| Normal | `x`, `X`, `d` and a motion, `dd`, `D` | Takes out the character, the one before, what the motion goes over, whole lines, or to the line's end. `diw` and `daw` take out the word, without or with its spaces. |
| Normal | `s`, `S`, `c` and a motion, `cc`, `C` | Takes out the same, and goes to insert mode in its place. `cw` changes the word, not the spaces after it. |
| Normal | `y` and a motion, `yy`, `Y`, `p`, `P` | Copies what the motion goes over, or whole lines, and puts what was copied after the cursor or before it; whole lines under or over its line. What is taken out or copied goes on the clipboard. |
| Normal | `u`, Ctrl-R | Undoes the last edit, and does it again. |
| Normal | `v`, `V` | Goes to visual mode, which selects characters or whole lines from the cursor, as the window's foot shows. Motions grow what is selected, `d` or `x`, `c` or `s`, and `y` take it out, change it or copy it, `v` and `V` switch or stop, and Esc lets it go. |
| Normal | Ctrl-C | Stops the agent. |
| Normal | Esc | Forgets a count typed. In the goal's, a sketch's or a rename's input, the settings or a project's card, it closes them as Esc does without Vim. |
| Normal | `j`, `k`, Ctrl-D, Ctrl-U, `gg`, `G` | Scrolls the conversation a line, half its height, or to its ends. |
| Normal | `j`, `k`, Ctrl-D, Ctrl-U, `gg`, `G` | In the settings or a project's card, scrolls them the same. |
| Normal | `/`, `?`, `n`, `N` | Finds text in the conversation forward or back, out of sight too, and again either way: scrolls to it and selects it. |
| Normal | `]]`, `[[`, `gy` | Selects the conversation's next entry, or the one before, from the first in sight, scrolling to it; copies what is selected of the conversation. |
| Normal | Ctrl-W `k` | Moves the keyboard up to the conversation, as to a window of its own (see below). |

**The conversation.** Ctrl-W `k` from the chat's input gives the
keyboard to the conversation, in normal mode, with a cursor on the
start of what is selected, or else of the first row in sight. The
cursor stays where it was when the keyboard comes back. It goes
through the text as the conversation draws it, a row being a line as
it wraps.

| Mode | Key | Does |
| --- | --- | --- |
| Normal | `h`, `l` | Moves the cursor a character left or right along the line. |
| Normal | `w`, `b`, `e` | Moves it by words, on from one piece of text, as a paragraph or a code block, to the next. |
| Normal | `0`, `^`, `$` | Moves it to the start, the first character that is no space, or the end of its row. |
| Normal | `j`, `k` | Moves it a row down or up, keeping to where along the row it was, on into the next entry. |
| Normal | Ctrl-D, Ctrl-U, `gg`, `G` | Scrolls as from the input; the cursor goes to the row nearest it in sight, the first after `gg`, the last after `G`. A cursor the mouse scrolls out of sight comes back into it the same way. |
| Normal | `/`, `?`, `n`, `N`, `]]`, `[[`, `gy` | As from the input; the cursor goes to the start of what they select. |
| Normal | `v`, `V` | Goes to visual mode, which selects characters from the cursor's, or whole pieces of text, as a paragraph or a code block. Motions grow it, `y` copies it, `ga` hands it to the chat, `v` and `V` switch or stop, and Esc lets it go. |
| Normal | Ctrl-W `j`, `i`, `a` | Gives the keyboard back to the input, in normal mode, or in insert mode at its cursor or after it. |
| Normal | Ctrl-W `k` | Opens the list of the project's chats, with the keyboard in it. Ctrl-W `j` or Esc closes it. |
| Normal | Esc | Forgets a count typed. It does not stop the agent. |

**The rail and the list of chats.** `j` and `k` move the selection,
`3j` three rows, `gg` to the first and `G` to the last. Enter or `l`
opens what is selected: a project's chat, or the chat, with the
keyboard in its input. `o` opens a new chat in the project selected.

**The stage's tabs, in normal mode.**

| Tab | Key | Does |
| --- | --- | --- |
| Code, a document, the tree, changes, a page | `j`, `k`, Ctrl-D, Ctrl-U, `gg`, `G` | Moves the cursor or the selection, or scrolls, a line, half the tab, or to its ends. In code, `12G` goes to line 12. |
| Code, a document, a page, a terminal | `/`, `?`, `n`, `N` | Finds text forward or back, as typed, in any case unless it has a capital, and again either way. |
| Code | `h`, `l`, `w`, `b`, `e`, `0`, `^`, `$` | Moves the cursor by characters and words, and to the line's start, first character and end. `j` and `k` keep the character it goes to. Resting on a name, the cursor shows what the language server says of it, as the mouse does. |
| Code | `%`, `{`, `}` | Goes to the bracket that matches the one under the cursor, or the next on its line, of `()`, `[]` and `{}`, and to the empty line before or after, or the count's. |
| Code | `*`, `#` | Finds the whole word under the cursor, or the next on its line, forward or back. `n` and `N` find it again. |
| Code | `zt`, `zz`, `zb` | Scrolls the cursor's line to the top, the middle or the bottom. |
| Code | `v`, `V`, `y` | Selects characters, or whole lines, from the cursor's, and copies them. Escape stops selecting. |
| Code | `ga`, `gd`, `gr`, `K`, Ctrl-O, Ctrl-I | Hands the selection or the cursor's line to the chat, goes to the definition of the name under the cursor, or of its line's first name, lists where it is used, shows at once what the language server says of it, goes back, goes forward. |
| A document | `]]`, `[[`, `y`, `ga` | Selects the next block, a paragraph, heading, list item, code block or table cell, or the one before, from the first in sight, scrolling to it; copies what is selected, or hands it to the chat. |
| The tree | `l` or Enter, `h` | Opens what is selected, or closes its folder. A file opened goes on with the keyboard. |
| Changes | `]c`, `[c`, Enter | Selects the next stretch of changed lines, across files, or the one before, with a count that many, and scrolls it to the top. Enter opens the file selected, or the file of the stretch selected at its first changed line. |
| A page | `f`, `;t` | Draws hints over the page; typing a label clicks it, or with `;t` opens the link on a tab of its own. Esc takes the hints away. |
| Every tab but a playground | `t` | Opens the stage's + menu. In that menu, as when the + opens it, words that are no address, file or path are searched for on a new tab with the [`search`](#settingsjson) engine, and `!command` runs the command in a terminal; the menu says what Enter does. |
| A page | `H`, `L`, `r`, `o` | Goes back, forward, loads again, takes the keyboard to the address. |
| A page | `gi`, `yy`, `i` | Gives the keyboard to the first field, copies the address, goes to insert mode. Esc in a field goes back to normal mode. |
| A terminal | `j`, `k`, Ctrl-D, Ctrl-U, `gg`, `G` | Scrolls back through what scrolled off, and to the end. |
| A terminal | `V`, `y` | Makes what `/` found, or what the mouse selected, whole lines, and copies what is selected. |
| A terminal, an Android device | `i` (and `a` in a terminal) | Goes back to insert mode. |

**A terminal and an Android device.** As in Neovim's terminal mode,
insert mode sends every key to the program, Esc, Ctrl-W and `:` too, so
Vim, a pager or the shell inside works; an Android device gets Esc as
Back. Ctrl-\ Ctrl-N goes to normal mode, where the keys go nowhere but
to the table above. Ctrl-\ then any other key sends that key, so
Ctrl-\ Ctrl-\ sends Ctrl-\. Typing in a terminal shows its end again. When the program ends, a shell's tab closes, the stage's last tab
excepted, and a command's stays to be read; the keys go to normal mode.

**The line at the window's foot.** `:` opens it for a command, `/` and
`?` for text to find. Tab and Shift-Tab, or Ctrl-N and Ctrl-P, go along
the list over it of what the line can go on to, Up and Down through
what was typed before, Enter runs it and Esc closes it.

| Command | Does |
| --- | --- |
| `:q[uit]` | Closes the tab the stage shows. |
| `:st[op]` | Stops the agent's turn in the chat shown. |
| `:new` | Opens a new chat in the project. |
| `:o[pen]`, `:e[dit]` | Opens a URL in the page shown, or a file. Other words are searched for in the page shown, and `!command` runs the command in a terminal. |
| `:t[abopen]` | Opens a URL or a file on a tab of its own, or searches for other words there. |
| `:set[tings]` | Opens the settings. |
| `:go[al]` | Sets the goal of the chat shown, or with nothing after it opens its input. |
| `:sk[etch]` | Starts a sketch of the chat shown. |
| `:re[load]`, `:ba[ck]`, `:fo[rward]` | Loads the page shown again, goes back, goes forward. |
| `:ch[at]` | Shows the chat of the project whose name fits. |
| `:pro[ject]` | Chooses the project whose name fits. |
| `:h[elp]` | Lists the commands. |

A name with `::` in it runs Cox's action of that name, as
`:workspace::NewChat`.

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
| `COX.md` | What every chat's agent is told first, before the project's `COX.md`. | [`cox.md`](../crates/coxd/src/agent/instructions/cox.md) |
| `sketch/start.md` | The first line of a sketch opened beside a chat. `{folder}`, `{from}` and `{what}` stand for the sketch's folder, what it starts from and what it tries. | [`sketch/start.md`](../crates/coxd/src/workflow/sketch/start.md) |
| `sketch/opened.md` | What an agent that ran a subagent in a sketch with `agent` is told about it. | [`sketch/opened.md`](../crates/coxd/src/workflow/sketch/opened.md) |
| `sketch/discard.md` | What a sketch's agent is asked before the sketch ends. `{left_out}` stands for the files too big to keep. | [`sketch/discard.md`](../crates/coxd/src/workflow/sketch/discard.md) |
| `sketch.json` | How sketches end. See [`sketch.json`](#sketchjson). | [`sketch.json`](../crates/coxd/src/workflow/sketch.json) |
| `wiki/save.md` | What "Save to wiki" asks the agent. | [`wiki/save.md`](../crates/coxd/src/workflow/wiki/save.md) |
| `close/merge.md` | What "Merge" asks the agent. | [`close/merge.md`](../crates/coxd/src/workflow/close/merge.md) |
| `icon/generate.md` | What "Generate", under the icon on a project's card, asks an agent. `{name}` stands for the project's name. | [`icon/generate.md`](../crates/coxd/src/workflow/icon/generate.md) |
| `subagent/reports.md` | What an agent that ran subagents with `agent` in the background is told, unasked, of those that ended their turn or came to ask you while it did not wait for them with `agent_report`: in one line, once its own turn is not running. `{reports}` stands for their reports, each as `subagent/ended.md`, `subagent/background.md`, `subagent/settled.md` or `subagent/asks.md` says. | [`subagent/reports.md`](../crates/coxd/src/workflow/subagent/reports.md) |
| `subagent/ended.md` | A subagent's report in `subagent/reports.md`, once it ended its turn. `{number}` and `{name}` stand for its chat's number and name, `{said}` for what it last said, cut to 2000 characters. | [`subagent/ended.md`](../crates/coxd/src/workflow/subagent/ended.md) |
| `subagent/background.md` | A subagent's report in `subagent/reports.md`, once it ended its turn while what it runs in the background, as a command it left running, goes on. `{background}` stands for what runs, beside what `subagent/ended.md` takes. | [`subagent/background.md`](../crates/coxd/src/workflow/subagent/background.md) |
| `subagent/settled.md` | A subagent's report in `subagent/reports.md`, once what `subagent/background.md` told runs in the background ended. It takes what `subagent/ended.md` takes. | [`subagent/settled.md`](../crates/coxd/src/workflow/subagent/settled.md) |
| `subagent/asks.md` | A subagent's report in `subagent/reports.md`, as it waits for you to answer. `{asking}` stands for what it asks, beside what `subagent/ended.md` takes. | [`subagent/asks.md`](../crates/coxd/src/workflow/subagent/asks.md) |
| `subagent/ci.md` | What follows a subagent's report in `subagent/reports.md`, once CI ended on a pull request it opened with `gh pr create`. `{url}` and `{title}` stand for the pull request, `{outcome}` for `passed` or `failed`, and `{failed}` for the checks that failed, if any. | [`subagent/ci.md`](../crates/coxd/src/workflow/subagent/ci.md) |
| `ci/ended.md` | What the agent of a chat you opened is told, unasked, once CI ended on a pull request it opened with `gh pr create`: in a line of its own, once its turn is not running, for it to go on, as to merge or fix what failed. `{number}`, `{title}` and `{url}` stand for the pull request, `{outcome}` for `passed` or `failed`, and `{failed}` for the checks that failed, with their links, if any. | [`ci/ended.md`](../crates/coxd/src/workflow/ci/ended.md) |
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
| `maps/<name>.json` | The project's maps; a repository of the project keeps its own the same way. See [Maps](#maps). |
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

### Maps

A map shows the project as boxes, from what it is among at the top,
through what is inside each box, down to the types of the code each box
owns. You and the agents keep its vocabulary in `maps/<name>.json`: the
boxes and the people and systems around them, where each sits on the
levels you place, the lines no code backs, and the code each box claims.
Cox reads the rest from an index of the code: each box's types, their
fields and functions, where each is used and what each calls, the lines
between boxes whose code meets, and the drift, where the vocabulary and
the code disagree, for you and the agent to talk through, then fix the
vocabulary, change the code, or agree to leave it in the vocabulary's
`agreed`. A box too big to read on one level of its types opens on a
level of boxes of its own instead, which you place, and which own its
code; drawn closed, it stands for them. Around the box of such a level
you place, in its `at`, the boxes of the levels above it is to show, and
the code's lines go on to those; place nothing around it, and the level
opens where its box sits in the level above, which stays around it,
moved aside to make room, and the code's lines go on to the boxes there.
Where you place some, an end of a line you drew that you placed nowhere
in the level, Cox places beside the box, on the side it lies on in the
level above. A card is one size, however long its name: a name of up to
16 letters stays on one line, a longer one takes two, and what fits on
neither is cut short. A box opened keeps the lines it
has closed: each goes on to the parts inside that the level inside joins
to the line's other end, or else to the box's edge, as a line from a
person does where no code ties it to one of the box's types. What
changed in the code shows on the boxes it falls in.

A line's end at a box can instead be a rule, which picks the types
inside the box the line goes on to as it opens, as
`{"from": "user", "to": {"box": "window", "implements": "Render"}}`.
`implements` picks the types that implement a trait or an interface, or
subclass a class, as a language server finds a type's implementations;
`uses` picks those whose declarations or functions name a type or a
function, as it finds references. The names are your code's and your
framework's, so the map names no type by hand that goes stale as the
code moves, and the rule reads the same in any language: from the
relationships and occurrences every index has, or, for Rust, from the
trait each `impl` block names. In a level of boxes the line goes on to
the boxes that hold the types picked; in a level of types, to the
types, those named most drawn whole and the rest shown when one end is
chosen. A rule that picks no type is drift. Write a name as it is
declared, not as it is imported or exported under another, as
`hono-base.ts/Hono` for the class `hono.ts` imports as `HonoBase`; and
with as much of its module before it as tells it from others of its
name, as `tui/Renderer`, whichever of `/`, `.` and `:` you part them
with. rust-analyzer names the trait of an `impl` block by its name
alone, so only a name's last part counts there. A rule sees only what
was indexed: code a build leaves out, as a Go file behind a build tag
or Rust behind a `cfg` the indexer did not set, is not there, nor are
types a Rust macro makes, as Cox indexes Rust without its procedural
macros. And it picks types only: a function that is no type's, as many
a handler or middleware is, is not an end it can reach. The agent's
`show_map` tool tells it how the vocabulary is written, opens the Map at
a box or a type, and answers what is there, with the drift. The
conversation keeps a row for each map the agent showed, which opens it
there again: on the stage, or on the phone, over the chat.

A map is told from whoever starts the project working: its drivers,
which the vocabulary's `top` names in `drivers`, as the people who use
the project, each by what they use it for, the systems that call into
it, and the clocks and events that set it off. Opened, the Map shows the
drivers alone; choose one, in the bar under the map's name or by its
card, and it shows what that driver sets working, and nothing else: the
boxes, the types inside them, and the people and systems around, placed
anew so that no room is left for the rest. "Everything" shows all of the
map, whoever starts it. A map with no drivers is shown whole.

What a driver sets working is what is reached from it. First along the
lines you wrote, each from its `from`, the end that starts it: through
the people and systems on the way, and into a box, to the parts the
level inside draws the line on to, by hand or by a rule, or else to all
of the box. The types the lines come to are where the driver comes in.
From there along the code, the way it goes: what a type's functions
call and name, what its fields hold, and on through the functions that
are no type's. And along a line you wrote from a box that owns code,
as from one process to another, once the code has come into the box. A
line from a box that opens on boxes is the part's that the level inside
draws it on from; drawn on from none, it is followed only where a
driver comes in at all of the box. So draw each line on, level by
level, from the part that starts it. Where one
driver's walk comes to a type another driver comes in at, it shows the
type, with a dashed edge, and goes no further from it: what happens from
there is the other's. So a type several drivers reach is in each one's
slice, and one driver's slice is not all of the map only because the
code is joined up somewhere. A type's detail names the drivers that
reach it, as "Reached from: User, Clerk", to tell whom a change to it
touches.

So write a line from the end that starts it: `both: true` only draws
the answer's way too. And give a line `stops: true` where what a driver
starts ends at the line's `to`, as from a build task to the app it
starts: the `to` is shown, closed. The page's address names the driver
first, as `#user/app/server`, and `*` there is all of the map; the
agent's `show_map` takes the same as `driver`.

The project's own maps are in `maps` in its folder. To keep a map with
the code it shows, versioned in the same commits, put it in `maps` in
the folder of one of the project's repositories: the project has it as
`<repository>/<name>`, as `app/use` for
`directories/app/maps/use.json`, after the project's own, and a chat that
works in a checkout of the repository, as a worktree, has it as `use`.
The `files` a map claims are from the folder its `maps` is in, as
`src/lib.rs` for `directories/app/src/lib.rs`; the modules it claims are
named as the index names them, wherever the map is. Cox reads a map's
vocabulary as the checkout has it, saved but not committed too.

`coxd` indexes the commit each of the project's repositories has checked
out, not what is not committed, by the files at the repository's top:

| Language | When the top has | Indexer |
| --- | --- | --- |
| Rust | `Cargo.toml` | `rust-analyzer`, as Cox downloads it for the [language server](#language-servers) |
| TypeScript | `tsconfig.json` | `scip-typescript`, run with `npx` |
| JavaScript | `package.json` and no `tsconfig.json` | `scip-typescript`, run with `npx` |
| Python | `pyproject.toml`, `setup.py` or `setup.cfg` | `scip-python`, run with `npx` |
| Go | `go.mod` | `scip-go`, which Cox installs with `go install` |

A repository of several packages may have these files in the packages'
folders only. Cox then indexes the folders its top names, all in one
index, with their paths from the top:

| Language | When the top has | Cox indexes |
| --- | --- | --- |
| Go | `go.work`, and no `go.mod` | the modules it uses, each a folder with a `go.mod` |
| TypeScript | `package.json` with `workspaces`, or `pnpm-workspace.yaml`, and no `tsconfig.json` | the packages they name, as `apps/*`, that have a `tsconfig.json`, each by its own |

A workspace none of whose packages has a `tsconfig.json` is indexed from
its top, as JavaScript. Code in a folder the top does not name so is not
indexed.

An index takes from seconds to minutes, as rust-analyzer does on a large
project; meanwhile the Map draws from the last commit's index, if there
is one, and says what it waits for, or what an indexer needs that is not
there. Cox keeps the indexes of the commits it drew last in
`cache/indexes` in its [data folder](#the-data-folder). rust-analyzer
runs the build scripts and builds the procedural macros of the code it
reads, the first time as long as a build: Cox keeps what it builds in
`cache/index-builds`, so that the next commit builds only what changed.

### Plugins

A plugin is a folder with an `index.html`, and optionally a
`plugin.json` naming its `title`. The stage's bar shows a button for it,
which opens it at `cox://<name>/`. The page reads what Cox knows as JSON
from `cox://api/...`, and draws in Cox's colors by linking
`cox://api/theme.css`, or `theme.css` beside its page, unless it has a
file of that name. Cox's own plugins, in
[`crates/cox/plugins`](../crates/cox/plugins), show how;
[`plugins.rs`](../crates/cox/src/stage/plugins.rs) lists what
`cox://api/` answers.

## Environment variables

| Variable | Meaning |
| --- | --- |
| `RUST_LOG` | What Cox writes to standard error. Errors only by default. |
| `COX_LOG` | What Cox keeps in its log files in `logs` in the data folder. Down to `debug` by default. |
| `COX_REGISTRY` | Another ACP Registry to fetch, such as a mirror. |
| `OTEL_METRIC_EXPORT_INTERVAL` | How often `coxd` sends its [telemetry](#telemetry), in milliseconds, when it is on. `60000` by default. |
| `COX_SERVER_<NAME>_VERSION`, `COX_SERVER_<NAME>_SOURCE` | The version, and the source, of the language server `<name>` that Cox gets. See [Where Cox gets a server](#where-cox-gets-a-server). |
| `ANDROID_HOME`, `ANDROID_SDK_ROOT` | The Android SDK whose emulator Cox runs. |
| `ANDROID_AVD_HOME`, `ANDROID_EMULATOR_HOME`, `ANDROID_USER_HOME` | Where Cox finds the Android virtual devices. |
