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
  "vim": true,
  "search": "https://duckduckgo.com/?q=%s"
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
| `vim` | `true` makes the keys work as in Vim. See [Vim's keys](#vims-keys). | `false` |
| `search` | The search engine: the address of its results, from `http://` or `https://`, with `%s` where the words go. Cox searches with it for the words typed in a page's address line, as a browser does (a `?` first searches for whatever follows), and after Vim's `t`, `:open` or `:tabopen`. One without `%s` is not used. | `https://www.google.com/search?q=%s` |

The settings card (Cmd-, or Ctrl-,) sets `agent`, `home`, `workflow`,
`wiki`, `translation`, `vim` and `search` for you, and leaves the rest of the file
as it is. `coxd` reads `phone` only when it starts.

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
| A page | `H`, `L`, `r`, `o`, `t` | Goes back, forward, loads again, takes the keyboard to the address, opens the stage's + menu. In the menu `t` opens, words that are no address, file or path are searched for on a new tab with the [`search`](#settingsjson) engine, and `!command` runs the command in a terminal; the menu says what Enter does. |
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
| `sketch/opened.md` | What an agent that opened a sketch with `open_chat` is told about it. | [`sketch/opened.md`](../crates/coxd/src/workflow/sketch/opened.md) |
| `sketch/discard.md` | What a sketch's agent is asked before the sketch ends. `{left_out}` stands for the files too big to keep. | [`sketch/discard.md`](../crates/coxd/src/workflow/sketch/discard.md) |
| `sketch.json` | How sketches end. See [`sketch.json`](#sketchjson). | [`sketch.json`](../crates/coxd/src/workflow/sketch.json) |
| `wiki/save.md` | What "Save to wiki" asks the agent. | [`wiki/save.md`](../crates/coxd/src/workflow/wiki/save.md) |
| `close/merge.md` | What "Merge" asks the agent. | [`close/merge.md`](../crates/coxd/src/workflow/close/merge.md) |
| `icon/generate.md` | What "Generate", under the icon on a project's card, asks an agent. `{name}` stands for the project's name. | [`icon/generate.md`](../crates/coxd/src/workflow/icon/generate.md) |
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
