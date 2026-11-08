# Contributing

## Commands

- `cargo xtask setup` installs the pinned development tools.
- `cargo xtask ci` runs every check. A change is ready only when it passes.
- `cargo xtask install` builds Cox for release and installs it apart from
  `target/`, with coxd started at login (see
  [Using Cox to build Cox](#using-cox-to-build-cox) and [coxd](#coxd)).
- `cargo xtask run` builds Cox and runs it with a fake agent, apart from
  what it kept before (see [A fake agent](#a-fake-agent)).

## Building on macOS

The Command Line Tools are enough: `cox` turns on gpui's `runtime_shaders`
feature, which compiles the Metal shaders when the app starts instead of
at build time, so Xcode's `metal` compiler is not needed.

## Building on Linux

`cox` uses [gpui](https://crates.io/crates/gpui), which links these
system libraries on Linux (Debian and Ubuntu package names):

```sh
sudo apt-get install libvulkan-dev libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev
```

`Cargo.lock` holds `libc` at 0.2.188: gpui depends on `xattr` 0.2, which
does not build with later `libc` releases on Linux. Do not update `libc`
until gpui moves off that `xattr`.

## The stage

The window's stage is a page rendered by CEF (Chromium) with no window of
its own, through the [cef](https://crates.io/crates/cef) crate. The crate's
build downloads the matching CEF distribution from the official builds
(about 200 MB, once per target directory).

`cox [--stage <url>] [--home <dir>] [--data <dir>] [<agent command>]` opens a window with
no chat in it. The rail on the left of the window shows the projects,
each as a folder, or the icon the human gave it, with its name under
it, and its folder's path where the mouse rests; the stage of the chat shown (see below) fills the
middle of the window, and the chat itself, its conversation, is in a
panel at the right. The head of the panel names the chat shown and
counts the other chats of its project that wait for the human; a click
on it opens a list of the chats of the project chosen on the rail, over
the conversation, which a click on one shows. A click on a project
chooses it, showing the chat of it shown last, or, with none, the stage
of its folder alone, as Cox shows its own, the home's, as it opens. The
right button on a project opens its menu: "Choose icon…" asks for a
picture, as a PNG, a JPEG, a WebP or an SVG, which Cox crops to a
square at its middle, or draws, for an SVG, and keeps, 256 pixels
square, in `state/icons` in Cox's data folder (see
[The stage](#the-stage)), named after the number the project is kept
by, so that the project's folder stays as it is; "Generate icon" has
an agent draw one, the agent a new chat talks to, in the project's
folder: not a chat, so not in the list, which reads the project, as the
judge of a goal does (see below), and answers with an SVG, which Cox
draws and keeps the same way, the square's edge dashed in the accent
meanwhile, and where the mouse rests, why when it drew none. What the
agent is asked is the workflow's `icon/generate.md` (see below), or else
Cox's own ([`workflow/icon/generate.md`](crates/cox/src/workflow/icon/generate.md)),
with `{name}`, the project's name. "Remove icon" takes the icon away. A project is a folder:
"Open folder…", the + at the foot of the rail (Command-O on macOS,
Control-O elsewhere), opens a chat in the project of a folder of your
choice, and "New chat" in the list opens another chat in the project
chosen. "New chat" (Command-N, Control-N)
opens one of Cox's own, in no project, which works in Cox's home: `<dir>`
when given, or else the folder the settings set, or else `~/cox`. The
rail shows Cox first, by its icon, with no name under it, as a place of
its own, whose chats are Cox's own. The gear at the foot of the rail (Command-, on macOS, Control-,
elsewhere) opens the settings, on a card over the window, where the
human sets the home: a path typed in its input, whole or from `~`, which
Enter sets, or a folder "Choose…" asks for. The settings are kept in
`settings.json`, in Cox's data folder (see [The stage](#the-stage)), as
`agents.json` is, and the
Cox's own chats that open from then on work there; `--home` still
goes ahead of them for its run, until the human sets the home. Under
the home, the card lists the agents of `agents.json` (see below), a row
each, with its name and its command on one line, as a shell takes it,
as `CLAUDE_CODE_ENABLE_TODO_TOOLS=1 npx -y
@agentclientprotocol/claude-agent-acp`, which Enter keeps, as the card
does when it closes; a dot marks the agent `settings.json` names, and a
click on another's names that one; "Test" starts the agent and asks it
to open a session, in a folder of its own, saying whether it works,
asks the human to log in first, or is not found on Cox's `PATH`; ×
takes it out of the list; and "Add agent" adds one of the human's own,
or one of the ACP Registry, as the registry runs it but for the version
it pins, for the human to change. The card also sets, as the home is
set, the folders of the human's workflow and wiki (see below), kept from
`~` when they are in the home directory, or, emptied, Cox's own; and the
translation (see below): the language, which, emptied, turns it off, and
what translates, the agent a new chat talks to, one of the human's, or
a command on one line, as an agent's. What else the files hold stays as
it is. What the card says is selected and copied as the conversation's
text is. Escape, the ×, or a click beside the card closes it.
Each chat starts its own
`<agent command>` working in its project's folder, and works on the
stage of that folder, which the chats working there share (see below),
showing `<url>` first, or else "Activity" (see below). Cox opened with no `<agent command>`, as the
Finder, the Dock or a launcher such as Raycast opens an app, starts the
agent `settings.json` names, by its name in `agents.json` (see below),
as `{"agent": "Claude"}`; with none named, or one `agents.json` does not
list, a chat says so as its agent fails to start. Cox reads
`settings.json` and `agents.json` again as they change, as when the
human edits them while it runs: a chat opened from then on, or an agent
chosen, is of the agents they list and name then, and a chat in no
project opened then works in the home they set, while a chat open goes on with its
agent as it was. On macOS, such a Cox
takes in the environment of the human's login shell first, `$SHELL -l
-i`, as their terminal has it, since launchd gives an app only
`/usr/bin:/bin:/usr/sbin:/sbin` as its `PATH`, where `npx` is not. Ahead of the first line sent in the chat, the
agent gets how the human works with Cox, whatever the project: the
`COX.md` of their workflow's folder, which `settings.json` names, whole
or from `~`, as `{"workflow": "~/dotfiles/cox"}`, or else `workflow`
in the home the settings set, as the wiki is (see below), or, when there
is none, Cox's own
([`cox.md`](crates/cox/src/agent/instructions/cox.md)), which says when
to use Cox's tools, as when to try work out in a sketch, since the tools
only say what they do. After it, when the folder has a `COX.md`, the agent
gets what it says. The chat says at its top which files it got. What Cox
tells the agent in the human's place at the turns of a sketch is the
workflow's too, in `sketch/` there, or else Cox's own
([`workflow/sketch`](crates/cox/src/workflow/sketch)): `start.md`, the
first line of a sketch opened beside a chat, in `{folder}`, which is `{from}`, to try `{what}`;
`opened.md`, what the agent that opened a sketch with `open_chat` hears of
it; and `discard.md`, what the sketch's agent is asked before the sketch
is discarded, with `{left_out}`, the files too big to keep. The list switches between chats (Control-Tab and
Control-Shift-Tab go along every project's), the stage staying as it
is while the chats work in one folder. The › at the head of the panel
folds it to the window's right edge (Command-\\, Control-\\ off macOS),
the stage taking its room, where a dot for each chat of the project
stays, which shows the chat, and the ‹ over them unfolds it. A chat is named by a number at first, then
after the first line sent in it, then by the title its agent gives its
session, if it gives one (ACP's `session_info_update`, as
`claude-agent-acp` sends), until the human renames it: a
double click on the head of the panel, or "Rename" in the menu of the
right button, on the head or on the chat's row in the list.
A dot on its row marks how the chat stands, and on the head for the
chat shown: a ring while its agent is
at work, in the accent, or idle, in grey; filled when it waits for the
human, in amber, or finished a turn the human has not seen yet, in the
accent. A closed chat has none. The project's square on the rail counts
its chats that wait for the human or that they have not seen.
On macOS, a chat the human is not looking at, as when another chat is
shown or Cox is behind other windows, tells them with a notification
when it comes to wait for them, and when its turn ends; a click on the
notification shows the chat. The Dock's icon counts the chats that
wait for the human or that they have not seen since a turn ended.
"Archive" in the menu takes a chat out of the list, to "Archived" at
its foot, from where a click brings it back; its agent, if it runs, goes on. A chat's agent runs in coxd (see
[coxd](#coxd)), which goes on after the window closes, until coxd is
stopped. Cox
keeps each chat's conversation, and each stage's tabs, as they change,
and lists the chats of earlier runs too, greyed and closed, for the
human to read what they showed. A stage opens again on
the pages and Markdown files it had; its terminals do not start again.
A line sent in a closed chat starts its agent again, which goes on with
the chat's session when it can (ACP's `session/resume`, or else
`session/load`); otherwise the agent starts a new session, and gets the
conversation so far, as Cox kept it, ahead of that line, and the chat
notes that the agent could not resume. A chat with no session to go on
with, as one that moved into a sketch or out of one, starts a new
session the same way, without the note.
Escape in a chat's
conversation, or its Stop button, stops the turn its agent is taking; the
conversation stays, and the next line can be sent once the agent has
stopped. A click on a link in a chat's conversation opens it on that
chat's stage.

A pill under the head of the panel, for the chat shown, shows what
its agent lets the human choose of its session, such as the model it
uses, its mode, or how hard it thinks: ACP's session config options, of
the kind that chooses one of a list, as the agent names them and in its
order, so Cox knows nothing of any agent's models. It shows what each is
set to, after a dot in the accent and the agent's name, as many as
fit in a narrow window, each whole, and a click opens a list of them, each
on a row; a click on a row shows what it can be set to, under the groups
the agent puts them in, with a line back to the rows; a click on one
sets it (ACP's `session/set_config_option`), while a turn runs
too, and the pill follows what the agent sets on its own. The chat keeps
how each is set, a closed chat too, where a choice waits for its agent
to start again, and sets its session so again when it does. A new chat,
the human's or one an agent opens, starts with the model, and what goes
with it (ACP's categories `model`, `model_config` and `thought_level`),
as the human chose last in any chat; its mode is its own.

The list starts with the agent the chat talks to, which a click on its
row chooses among the agents Cox knows, while no turn runs: first the
`<agent command>`, which a new chat talks to, or, with none, the agent
`settings.json` names, then those the human lists
in `agents.json`, in Cox's data folder (see
[The stage](#the-stage)), as `[{"name": "Codex", "command": ["npx", "-y",
"@agentclientprotocol/codex-acp"]}]`, with `NAME=value` ahead of the
program setting its environment, as on the command line, then, under
"ACP Registry", the agents of the
[ACP Registry](https://agentclientprotocol.com/get-started/registry)
that `npx` or `uvx` runs, or whose program Cox downloads: one the
registry lists for the platform Cox runs on, with the SHA-256 of its
archive, since an archive Cox cannot check is not offered. Such an
agent's program is downloaded the first time a chat starts it, which
the chat notes, checked by its SHA-256, and unpacked in
`cache/agents/<id>/<version>` in Cox's data folder, from where every chat
starts it after; an archive of another SHA-256 is refused, and the
agent does not start. Cox fetches the registry as it starts, and keeps
it, as `cache/registry.json` there, for a run that cannot
fetch it; `COX_REGISTRY` names another registry to fetch, as a mirror
of one's own.
An agent of the registry that runs the package one of the human's runs
is left out, and the human's is named as the registry names it. Chosen,
the agent starts at once, in a session of its own, in place of the
chat's agent, which stops, and the chat notes the switch. The new agent
is told the conversation so far ahead of the human's next line, as an
agent that could not resume is, with where the chat switched from one
agent to another, so that it knows which replies were another's. The
chat keeps its agent, a chat its agent opens talks to the same agent,
and the settings a new chat starts with, as the human chose them last,
are of each agent apart.

An agent that has the human log in first says so, with ACP's
`authRequired`, as it opens its session or as it takes a turn, and the
chat asks the human on a card in the conversation, with the ways to log
in the agent offers (ACP's `authMethods`), by their names: a click on
one, or its number, logs in that way, and the agent opens its session
then, or the line goes again. A way the agent carries out itself, as
Codex's API key, or its login to ChatGPT in a browser, Cox asks the
agent to take (ACP's `authenticate`); a way carried out in a terminal,
as `claude-agent-acp`'s logins to Claude, runs the agent's command, with
the way's arguments and environment, in a terminal on the stage, where
the human answers what it asks, and once it ends, Cox asks the agent
again, on the same connection. The card says why the way tried last
failed, when it did. Escape gives up: the turn fails, or, before the
session opens, the agent closes.

A chat can have a goal, which the chat works until it is met: a
command, such as `cargo xtask ci`, which has to pass, or words, such as
"The greeting names the user.", which an agent judges. "Set goal…" in
the menu of the right button opens an input above the chat's own for
the goal (Enter sets it, Escape leaves it as it was, and emptied, it
clears the goal), where `$ ` and a command is the command, and any
other text is a goal in words; "Clear goal" clears it; its row in the list marks a
chat with a goal (◎), and a bar at the top of the conversation names
the goal, with, while the agent works toward it, a mark for each of the
turns a line gets: those whose check failed, the one it takes, and the
rest. When a turn ends as the agent ends it, a command runs
in a terminal on the stage, as the agent's `run_command` tool would.
While it fails, Cox sends the agent what it printed, as the next turn,
shown in the conversation in the human's place but dimmer, and the
agent goes on. The chat stops to wait for the human when the command
passes, or after ten turns on one line of theirs, or when the agent
made no tool call for two turns in a row, which says it has something
to ask, or when Escape stops the check. The goal lasts across runs,
with the rest of the chat.

A chat's agent can propose a goal too, with Cox's `propose_goal` tool,
as the human and it talk through what they want, or when the human asks
it for one: a command or words, as "Set goal…" takes them, and why. The
goal shows on a card in the conversation, with "Set goal", which sets
it, "Change…", which opens the input above the chat's own on it to
change before Enter sets it, and "Dismiss", which lets it be; the card
then says which the human chose. Only the human sets a goal so: the
agent hears at once that the goal is for the human to set, and which
goal the chat has then, and goes on with its turn.

A goal in words is judged by an agent Cox starts for it, of the same
command as the chat's, in the project's folder: not a chat, so not in
the list, and the human does not talk to it. Each time a turn ends, Cox
asks it whether the goal is met now, telling it what the chat last
said, and it ends its reply with `PASS`, or `FAIL: ` and why. While it
fails, the chat's agent is told why, as the next turn, as with a
command's output, and when it passes the chat notes so and waits for
the human; a reply with no verdict counts as a fail. The judge is told
to read the project and run what reads or tests it, but to change
nothing, and gets none of Cox's tools, nor the stage's; of its
permission requests, Cox allows those that read or search, by the kind
of tool ACP says the call is, and those that run the usual tests
(`execute:git diff*`, `execute:cargo test*`, and so on), and cancels
the rest, since no human answers for it. The judge
keeps one session while the goal lasts, so it remembers what it looked
at, until the goal changes or Cox quits. A `.cox/goal.md` in the
project's folder replaces what the judge is told it is, for a project
that wants its goals judged in a way of its own; Cox still adds how to
end the reply, and the goal. When the judge finds that one command
decides the goal, passing just when it is met, it names the command on
a line `COMMAND: ` ahead of its verdict, and the chat proposes the
command as the goal, on a card as the agent's proposals are, once: set,
the command checks the goal from then on, the same way each time, and
the judge goes.

What else is to run as a turn ends, whatever the chat, is a trigger: a
Markdown file of the human's workflow, in `triggers`, which runs in
every project, or of a project, in `.cox/triggers` in its folder,
which the project's repository can keep, named after it, as
`.cox/triggers/ci.md`. Its properties, the front matter between `---`
lines at its top, say when it runs and who runs it, and its rest is
what is done:

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

`on` is the event it runs on, or a list of them: `turn.end`;
`goal.met`, as a chat's goal is met; `wiki.written`, as a chat's agent
writes a page of a wiki, or, as `wiki.written: {page: "research/*"}`,
one of the pages the globs name; `sketch.end`, as a sketch ends, in the
chat that goes on with it; `chat.open`, as a chat opens, once its agent
has started; or `schedule`, which opens a chat at the times it says
(see [Schedules](#schedules)). `when` narrows it:
`changed`, globs of the chat's folder, those that start `!` leaving
files out, which run it only when a file they match changed since it
last ran in the chat, or else since the last commit, as Git tells,
comparing the trees of what the folder holds then and now, without
touching its index; and `sketch`, `skip` not to run in a sketch, or
`only` to run in one alone. `by` is who runs it: `shell` runs the rest
as a command, in a terminal on the stage, as a goal's, which passes
when it exits 0; `this` tells the chat's own agent the rest, as the
next turn, in the human's place, once the triggers that run with it
went, and, as a turn ends, once for a line of the human's, so that the
turn it starts does not tell it again; and `new` asks an agent of its own, of the same
command as the chat's, in the chat's folder, as a goal's judge, with
none of Cox's tools nor the stage's, which ends its reply with `PASS`,
or `FAIL: ` and what the chat has to act on; `allow` and `reject` are
the patterns of its permission requests Cox answers in the human's
place, as `open_chat`'s, what a judge may unless it says. The agent
keeps one session while the file stays as it is, so it remembers what
it looked at.

When a turn ends as the agent ends it, the goal is checked first, then
the triggers of the chat's folder run in order, the project's and then
the human's, each by name, unless the human ordered them otherwise. The
first that fails stops them, and the chat's agent is told what it
found, as the next turn, in the human's place, as a goal not met tells
it, the turns counting toward the ten of a line of the human's; the
chat waits for the human once they all pass, or one could not run. A
line names, under `TURN END`, the goal and each trigger, with a dot of
how it went: a ring while it waits or runs, then in the accent once it
passed, in amber when it found something, and in red when it could not
run, as when its agent did not start; a click on one opens what it
said. A trigger Cox cannot read is said in the log, and does not run.

The triggers of the other events of a chat run once the chat waits for
the human, as after the turn its goal was met in, or the turn its
agent wrote a page in, all of them, in the same order, on a line of
their own, under what happened, as `GOAL MET`, `WIKI WRITTEN`, `SKETCH
ENDED` or `CHAT OPENED`, with a dot each as at a turn's end: what they
find stays there, and the chat's agent is not told it. An agent of
such a trigger, `by: new`, is told what happened, as which page was
written.

"Triggers", a tab of the stage (see below), lists the triggers of the
stage's folder, under the event they run on, in the order they run in
there, each with the dot of how it last ran in a chat there, which says
so and when where the mouse rests, its name, what it does, when it ran,
whether it is the project's or the human's, and a switch that turns it
on or off in that folder alone; a drag of its row orders them there,
and a click opens its card, as "New trigger" at its top does for a
trigger of the stage's folder that is not there yet. The card is what
the file says, for the human to change: its name, whether it is the
project's or the human's, which a click on it switches, the event it
runs on, what narrows it, as the pages of the wiki it runs on, who runs it, the patterns its agent is
allowed and refused, each with its × and a + for another, and what is
done. The part the human works on is open, and the other folded to a
line, a click on which opens it: what is done, for an agent's, and the
rest, for a shell's or a new one. What the human changed is marked, in
amber, until they save it, and the card's foot counts it, as "2 unsaved
changes", with "Revert", which takes it back, "Try on the last turn",
which runs the card as it stands on what a chat in the folder said
last as its turn ended, as it would run then whatever changed, and
shows how it went under it, and "Save", which writes the file, and
moves it when its name or its place changed; nothing changes the file
until then. "Open file", with nothing changed, opens the file on a tab.
"Discuss" opens a talk beside the card with an agent, the one a new
chat talks to, in the folder: not a chat, so not in the list, which
reads the project and the trigger's file, and is told what the card
says, how the trigger last ran and how the card went tried, with each
line; it changes nothing, but proposes the card whole, which is laid on
the card, its changes marked as the human's are, for the human to save
or not. The card closing ends the talk. In the home, it lists those
of each project too, under its name. Those on a schedule are listed
there too (see [Schedules](#schedules)). Cox keeps which are off and their
order apart from the files, in `state/triggers.json` in Cox's data
folder, how each last ran in `state/trigger-runs.json`, and what a chat
of each folder said last as its turn ended, for the card to be tried
on, in `state/last-turns.json`, so that a file stays as it is written.

A chat's agent can open a chat too, with Cox's `open_chat` tool, giving
it its first line: the chat opens in the list after the agent's own,
marked ↳, in the same project, and works on that line, which shows in its
conversation in the human's place but dimmer, after a line that says
which chat opened it. The agent says how the chat's agent is set up:
`context`, text the agent gets after the project's `COX.md` and ahead of
the first line, which the chat notes at its top; `tools`, the names of
the Cox tools it gets (`show_markdown`, `show_file`, `show_changes`, `show_map`, `run_command`,
`open_chat`, `wait_chat`, `propose_goal`, `start_sketch`, `wiki_search`, `wiki_read`, `wiki_write`,
`raw_keep`, `raw_search`, `raw_read`, and on macOS `mac_windows`, `mac_window_look`,
`mac_window_click`, `mac_window_type`, `mac_window_key`, `mac_window_scroll`, and `android_devices`,
`android_look`, `android_tap`, `android_swipe`, `android_type`, `android_key`), all of them unless given; `stage`, `drive` for the tools
that read and drive the stage, the default, or `look` for none of them;
`allow` and `reject`, patterns of the permission requests Cox answers
in the human's place, a request both match being rejected, and the
rest asked of the human as usual, with the conversation saying which
Cox answered; and `goal`, what the chat works until it is met,
as "Set goal…" takes it: `$ ` and a command, or words; and `sketch`,
which makes the chat a sketch, from the project (`project`) or from
nothing (`empty`). A pattern matches the title of the call the request
is for, where `*` stands for any run of characters: the agent's title,
as the conversation shows it, as `Edit src/main.rs`. Since each agent
titles its calls its own way, a pattern can start with one of the kinds
of tool ACP sorts the calls into, whatever the agent names its tools,
and a `:`: `read`, `edit`, `delete`, `move`, `search`, `execute`,
`think`, `fetch`, `switch_mode` or `other`. It then matches the calls
of that kind whose titles match the rest, as `edit:*` for every edit,
or `execute:cargo test*` for a command whose title, as agents title
commands, is the command. A
chat the human opens is set up with none of this: its agent gets every
tool, and the human answers every request. Which chat opened a chat, and
how its agent is set up, last across runs with the rest of the chat.
A sketch tries the work out apart from the project, before the human and
the agent agree what it is to be. A sketch is a chat of its own,
beside the chat it is opened from, so that what it tries, and its
stage, stay apart from the work the chat goes on with in the project. A
chat's agent opens one with Cox's `start_sketch` tool, saying what the
sketch starts from, `project` or `empty`, and what it tries: the sketch
opens under the chat, marked ↳ as a chat an agent opens is, named after
what it tries, its agent set up as the chat's, and told what the chat
talked through so far, as an agent that could not resume is told the
conversation, and then what it tries; the chat notes it, and its agent
hears the number the sketch goes by, for `wait_chat`. `sketch` in
`open_chat` opens a sketch too, on a line of the agent's own, for trying
several ways side by side, one sketch each. The human opens one with
"Start sketch…" in the menu of the right button, which opens an input
above the chat's own, where the human writes what the sketch is to try,
as what to find out, and a click on "from the project" or "from
nothing" beside it chooses what it starts from (from the project at
first, when the project is in a Git repository). Enter opens the sketch,
as `start_sketch` does, and shows it, and Escape leaves the chat as it
was. A sketch opens no sketch beside it. A sketch's agent works in a
folder Cox makes for it in `sketches`, in Cox's data folder (see
[The stage](#the-stage)), and changes and tests the files there
without asking the human, the policy allowing the calls the workflow's
`allow` names (see below), as Cox's own those that edit, delete or move
files (`edit:*`, `delete:*`, `move:*`), when every file the agent says
the call works on is in the sketch's folder, and what the judge of a
goal may run; a call that changes files elsewhere, or
does not say which, is asked of the human. A sketch from the project works in a Git worktree of
the project's last commit, so the project has to be in a Git
repository; a sketch from nothing works in an empty folder, `scratch-`
and a number, made a Git repository of its own, for a project of any
kind or none, which "Open folder…" opens as a project once the human
wants to go on with it. The chat notes where it works from then on,
and the agent that opened a sketch with `open_chat` is told, to read
what the sketch made and propose a goal from it. A chat a sketch opens works in the sketch's
folder too, and a sketch goes on there once it is started again. The
menu of the right button on a sketch ends it once the human has what
they wanted of it, in the ways the human's workflow says, in its
`sketch.json`, or else as Cox's own
([`sketch.json`](crates/cox/src/workflow/sketch.json)) does: "Open as
project", for a sketch from nothing, opens its folder as a project, with
a chat in it, to go on with what it made; and "Discard sketch" takes its
folder away, unless it was opened as a project, which the human goes
on with. Before it does, once the turn that goes on has ended, Cox asks
the sketch's agent, in the human's place, to sum the sketch up in the
conversation, for it to be done again from the conversation alone:
what it tried, how, the commands it ran and what they showed, what it
found, and the changes that matter, as code; Escape stops that, and
the sketch goes as it is. What the sketch changed since it started is
kept too, as one commit on where it started, apart from the commits
the sketch made itself: `refs/cox/discarded/` and the name of its folder
in the project's repository, for a sketch from the project, which
`git show` reads, and for a sketch from nothing, a Git bundle in
`sketches/discarded`, which `git clone` makes again. A file over 256 KB,
or past 1 MB with the files kept with it, the smaller kept first, is
left out: the agent is asked how to make it again, and the chat notes
its path, its size and the hash Git names it by. A sketch ended stops
its agent, and those of the chats it opened, which worked in its
folder, and each chat goes on in the project: a line sent starts its
agent again there, with the conversation so far, asking the human as
any chat does, since what it was allowed was for the sketch. The chat
the sketch was opened beside notes how the sketch ended, and goes on with
it, as the ending says (see below); a sketch opened beside no chat, as
one of an earlier Cox, goes on itself.

`sketch.json` names, as `allow`, the patterns of the calls a sketch's
agent makes unasked, as `open_chat`'s `allow` takes them, and, as
`endings`, the ways a sketch ends, in the order the menu offers them.
Each has its `name`, as the menu shows it, and says what it does: `from`,
`project` or `empty`, for an ending of those sketches only; `sum_up`, a
text of the workflow, from its folder, which the sketch's agent is asked
first, as `sketch/discard.md`, with `{left_out}`; `keep`, as `"ref"`, to
keep what the sketch changed as discarding does; `folder`, `"open"` to
open a sketch from nothing as a project, in place of its folder going;
and, for the chat the sketch was opened beside, which goes on in the
project, `goal`, as "Set goal…" takes it, and `then`, a text of the
workflow, which its agent is told at once, in the human's place, with
`{kept}`, the ref or the bundle what the sketch changed is kept in, and
`{summary}`, what the sketch's agent said last, as its summing up. So a human who sketches first and
rewrites after can end a sketch with "Rewrite in project":

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

A `sketch.json` Cox cannot read is said in the log, and Cox's own counts
in its place.
`open_chat` answers with a number the chat goes by for this run, which
the agent gives Cox's `wait_chat` tool to wait for the chat's turn to
end, or for the chat to come to wait for the human, and hear how it
stands then: what its agent last said, or what it asks the human. A
chat that is not at work answers at once.

The human keeps a wiki, one for all their projects, of what the agents
learned: Markdown pages by topic, as `gpui-layout.md` or
`rust/errors.md`, in the folder `settings.json` names, whole or from
`~`, as `{"wiki": "~/notes/cox"}`, an Obsidian vault or a Git repository
of theirs, or else `wiki` in the home the settings set, as
`~/cox/wiki`, where the human sees it and every Cox of theirs finds it
(`--home` is for its run, and does not move it). A chat's agent
searches it with Cox's `wiki_search`, for the pages that have every word
it looks for, reads a page with `wiki_read`, or the index without one,
and writes a page whole with `wiki_write`, through Cox, which adds,
under the page's "Used in", where it was used: the day, the project,
the chat, by its name and the number it is kept by, the commit its
folder is at, and the paths of the code the agent gives, so that a page
read from another project says where it came from. Cox keeps
`index.md`, a line for each page with its title and its first line,
unless the human wrote an index of their own. Cox's own `cox.md` tells
the agent to search the wiki before it works out what another project
may have met, and to write to it only when the human asks; "Save to
wiki" in the menu of the right button asks, in the human's place, as
the workflow's `wiki/save.md` says, or else Cox's own
([`workflow/wiki/save.md`](crates/cox/src/workflow/wiki/save.md)): to
keep the decisions and why, what failed and why, and what a tool needs,
not how the code works, and no secrets.

A project has a wiki of its own too, for what holds only of it, as of
its own people, customers or plans: pages written the same way, in
`.cox/wiki` in its folder, which the project's repository can keep,
with an `index.md` of its own. Only that project's chats find its
pages, read them, or write them: `wiki_search` answers the project's
pages, then the human's, `wiki_read` reads the project's page of a
name before the human's, unless asked for the human's, and the indexes
of both without a page, and `wiki_write` writes the page where it is,
or, with `keep_in_project`, in the project's wiki, or, set false, in
the human's.

So that the human need not remember to, Cox asks at a chat's
milestones, when its goal is met and when a sketch ends, on a card in
the conversation of the chat they go on in, whether to keep what was
learned in the wiki: "Save to wiki" asks the agent as the menu's does,
for a sketch the sketch's own agent, which found it, and goes on in the
project, and "Not now" lets it be. A goal is asked of once for a line
of the human's, so that the turn that keeps it, which meets the goal
again, does not ask again; and a chat whose agent has no `wiki_write`
is not asked.

"Knowledge" on the stage's bar, a plugin (see below), at
`cox://wiki/`, shows the human what the agents learned and read, on
shelves the line at its top chooses between: "Your wiki", the human's,
"This project's wiki", the project's own, at `cox://wiki/#project:`,
which the home has none of, and "Sources", what the agents kept as
they read it (see [Research](#research)), at `cox://wiki/#raw:`, each
with how many it holds. A wiki shows its pages, with their titles, their first
lines and where they were used last, which a line at the top narrows to
those that have every word typed; and a page, at `cox://wiki/#` and its
name, as `cox://wiki/#rust/errors`, or `cox://wiki/#project:` and its
name for the project's, drawn as Markdown, its links going
to the pages they name on its shelf, `[[name]]` or `[[name|what it says]]`, as
Obsidian writes them, or a Markdown link to a page's file, a name that
is no page's naming the only page whose last part it is, as Obsidian
finds it; then where it was used, the pages that link to it, and its
file, which a click opens on a tab. HTML a page holds shows as text,
so that what an agent wrote runs nothing on the stage. Cox marks, with
no agent, the pages with links to no page, those no page links to, the
index aside, those with no title, and those that say nowhere where they
were used, as those the human wrote; chips over the pages narrow them
to those of a mark. The sources are listed the last kept first,
each with its title, where it was read, the day it was kept and the
chat that kept it, which the line at the top narrows too; and a source,
at `cox://wiki/#raw:` and its name, drawn as a page is, with where it
was read, a page of the web on a tab of its own, and the day and the
chat that kept it. The page reads them all again every two seconds.

The agents write the wiki in English, whatever language their chats
speak, as Cox's tools and `wiki/save.md` tell them, so that a page is
found by the same words from every chat; the human reads it, and
whatever else they select in the window, in a language of their own,
as `settings.json` names it, as `{"translation": {"language":
"Japanese"}}`. "Knowledge" then shows a page, or a source, in that language, as far as it
is translated, the rest as it is written, with how far it has gone,
and a chip "As written" over it to read it as the agent wrote it. It
is translated a block at a time, its paragraphs, headings, lists,
quotes and tables, as it is opened, and not its code or the HTML it
holds. Text selected anywhere in the window, in the conversation, an
input, the settings, a tab of code or Markdown, a terminal, or a page
of the stage, is translated with "Translate" in the menu the right button opens on
it, with "Copy", a tab of code's own menu having "Translate", and Cox's
menu showing on a page in place of the browser's; or, but on a page,
with Command-Shift-T (Control-Shift-T off macOS). A card by the mouse
shows the translation as it arrives, as far as the command or the
agent has written it, which "Copy" copies once it is whole, and
Escape, the ×, or a click beside it closes. Text in
English goes into the human's language, and the rest, as text mostly
of other letters, into English. Each translation is kept, by what it
says, the way it goes and what translated it, in `cache/translations` in Cox's
data folder, so that a page that changed is translated again only where
it changed, and the wiki's folder holds only what was written.
What translates is the human's to choose: `command`, a command that
reads the text on its standard input and writes its translation on its
standard output, with `{from}` and `{to}` in its arguments standing
for the languages, as a model run on the machine, such as PLaMo 3
Translate through
[plamo-translate-cli](https://github.com/pfnet/plamo-translate-cli),
which `plamo-translate server` keeps loaded between texts:

```json
{"translation": {"language": "Japanese",
  "command": ["plamo-translate", "--from", "{from}", "--to", "{to}"]}}
```

or else `agent`, an agent Cox knows, by its name, as `"Codex"`, or,
unless given, the agent a new chat talks to, which Cox asks over ACP,
as it does the judge of a goal: not a chat, with no tools, its
permission requests refused. A page's translation that fails says why
over the page, and is not tried again until `settings.json` says
another way; a selection's says why on its card.

The conversation runs down a line on its left, which names who speaks
where they start, with a dot: the human (YOU), Cox in their place
(COX), or the agent, by the name the human knows it by; and marks in
amber the card of what the agent asks the human, whose first option,
filled, stands out. The conversation shows each tool call the agent
makes on a line of its own, marked as running, done or failed; a click
on the line opens what the call showed, which shows from the start for
a call that failed.
"flow" at the end of the line opens, on the stage, how the call went
through Cox, read from what Cox kept of the run (see
[What Cox keeps of each run](#what-cox-keeps-of-each-run)): the
agent's process, Cox's thread for the agent, the chat's tool server and
the window as lanes, and each step as a row, with how long after the
first it came, an arrow into the lane it is taken in, what was said or
done, and where in Cox's code it is taken, as `asked.rs:67`, which a
click opens when the project has the file, as it does when the project
is Cox itself. A step in a file that changed, as "Changes" shows it, is
marked. For a call of one of Cox's own tools, the agent says over
ACP that it calls the tool, `cox --mcp` passes its `tools/call` on to the
tool server, the tool asks the stage, the stage carries it out, the
server answers, and the agent says the call is done, the flow telling
the call's `tools/call` from others by its arguments, which the agent
says over ACP as the call's `rawInput`, or, when it does not say them,
by when it came; for another tool,
such as the agent's own `Read`, the flow shows only what the agent says
of it, and its permission request and Cox's answer when it asks one.
What the agent thinks on its way, when it sends it, shows as an aside,
its start in italics on a line of its own, which a click opens to all
of it, and open while it arrives, and so does the plan it works to,
with each step marked as waiting, under way or done. The plan also
stays in sight on a card over the top right of the conversation, after
its turn too, until the next line is sent; a click on the card's line
folds it to that line. The tool calls, thoughts and
plan of the agent one after another fold under one line, which names
the agent where it starts to speak and counts the steps, its tool
calls, as `CLAUDE · 4 STEPS`, and how far the plan has gone: each call
shows under it as a chip with its title, short, and a mark of how it
goes, a ring while it runs, then a dot, in the accent once done, or in
red, on red, when it failed; a click on a chip opens the call's line
under the chips, and the last thought shows under them, while it
arrives too, until a click on the line opens all of the work, each on
its line. When the agent asks permission, a click on an option,
or its number, answers. The input, while it is empty, tells what the
chat waits for, while it is not the next line to send, or else that it
waits for a reply.

The inputs, the chat's and the others, such as the address of a page,
edit as a text field of the platform does: the cursor over them is an
I-beam, as it is over the conversation's text, a tab of code and a
terminal; a double click selects the word under the mouse, or the run of
spaces or punctuation, and a triple click the line, a drag from either
going on by words or lines; Option-Left and Option-Right go a word back
and on, Command-Left and Command-Right to the start and the end of the
row, Command-Up and Command-Down to those of the text, Option-Backspace
and Option-Delete take out a word, and Command-Backspace the row up to
the cursor, with Shift selecting as they go, and Control-A, Control-E
and Control-K as in Emacs; off macOS, Control does what Option does,
and Control-Home and Control-End go to the start and the end of the
text. On macOS, a key with Control that is none of these, nor any of
Cox's, goes to the input method, as macSKK's Control-J, which turns its
kana on from its ASCII mode.

An agent can run commands of its own, as `claude-agent-acp` runs Claude
Code's `/compact` and `/review`, when a line starts with `/` and the
command's name, with what follows the name as the command's input, and
tells the chat which it has (ACP's `available_commands_update`). While
the line is `/` and the start of a name, a list over the input shows the
commands that fit, the names that start so first, each with what it
does and what to write after it: Up and Down choose one, Tab or Enter
put it in the line, as a click does, Enter sends a line that names it
whole already, and Escape closes the list until the line changes. The
chip "/ commands" under the input starts an empty line with `/`, while
the agent has commands. A line that runs a command is sent as the
prompt's first block, which is how the agent knows it: what goes ahead
of the first line, as the project's `COX.md`, waits for the next line
instead.

A line points the agent to files of the project with `@` and the file's
path from the chat's folder, at the start of a word, as
`@src/main.rs`, or `@"docs/a file.md"` for a path with a space. While
the word at the cursor is `@` and the start of a path, the list over the
input shows the project's files that fit, without what Git ignores: the
letters typed have to come in the path in their order, as `@chv` finds
`crates/cox/src/chat/view.rs`, and the paths where they start words,
follow each other, or fall in the file's own name come first; with
nothing typed yet, the files that changed, as "Changes" shows them, come
first. The keys are the commands' keys, and the chip "@ files" under
the input starts a path at the cursor with `@`. The line goes to the agent as
it is written, and after it each file it points to that is there: the
file's text, when the agent takes files whole (ACP's
`embeddedContext`), as `claude-agent-acp` does, and the file is text of
256 KB or less, or else a link to the file (ACP's `resource_link`), for
the agent to read.

The human hands the chat shown what they selected elsewhere on the
screen, in its conversation, a tab of code or Markdown, a terminal, a
page of the stage or the settings, with "Add to chat" in the menu the
right button opens on it, beside "Translate", or, but on a page, with
Command-L (Control-Shift-L off macOS, where Control-L is the
terminal's). It shows as a chip over the input, named by where it is
from, as `main.rs:5-7` for the lines of a file, a page's title, what a
terminal runs, or the start of what was selected in the conversation,
with where it is from and the start of it where the mouse rests; a
click on a chip of a file or a page opens it, and its × takes it out.
The chips go with the next line sent, and stay under it in the
conversation; while the agent takes a turn, they wait, as the line
does. Each goes to the agent after the line, as text with where it is
from, as a URI: the file's, with the lines, as
`file:///work/src/main.rs#L5-7`, the page's address, `cox://terminal`,
`cox://settings`, or `cox://chat/` and the chat's number: when the
agent takes files whole, as a resource of that URI (ACP's
`embeddedContext`), and else as text after a line that says where it is
from.

The human hands the chat shown a region of the window too, as a
screenshot is taken, to show the agent what they see where text says it
poorly, as a layout gone wrong, or to point at a part of a page:
Command-Shift-L (Control-Alt-Shift-L off macOS), or the chip "▭ region"
under the input, lays a sheet over the window, where the human drags a
rectangle over what to hand, and Escape, or the right button, gives up.
The region shows as a chip over the input, as what is selected does,
named after the parts of the window it covers, as `▭ main.rs:16-20`,
and goes to the agent after the line, as the selections do, under
`cox://region`: where in the window it is, and what each part it covers
says of it: a tab of code, the lines in it and their text; a tab of
Markdown and the conversation, the text in it; a terminal, the rows
and columns in it and what they show; a page, its title, its address
and where on the page the region is, in CSS pixels of the page's
viewport; an Android device's display, or a Mac window, where on it,
in the pixels the agent's tools take; and then its picture, which Cox
takes of the screen as the sheet goes, and keeps in `state/pictures`
in Cox's data folder (see [The stage](#the-stage)): the picture itself,
when the agent takes pictures (ACP's `image`), as `claude-agent-acp`
does, or else a link to its file. Taking the picture is through
`screencapture` on macOS, which has to be allowed to record the
screen, and through ImageMagick's `import` on Linux, as `cox --inspect
screenshot` takes it (see [Asking Cox how it stands](#asking-cox-how-it-stands)).

The plan and the thoughts are ACP's `plan` and `agent_thought_chunk`
updates, which Cox shows from any agent that sends them. How an agent
is to be run to send them is the agent's: `claude-agent-acp` sends, as
the plan, the tasks Claude Code keeps with its task tools, which recent
models get only when the agent runs with
`CLAUDE_CODE_ENABLE_TODO_TOOLS=1`, and sends what Claude thinks only
with `CLAUDE_CODE_THINKING_DISPLAY=summarized`, since recent models
leave it out unless asked. So, to run Cox with Claude Code, for both:

```sh
CLAUDE_CODE_ENABLE_TODO_TOOLS=1 CLAUDE_CODE_THINKING_DISPLAY=summarized \
  cox npx -y @agentclientprotocol/claude-agent-acp
```

A chat's stage holds pages in tabs, in the middle of the window, beside
the chat's panel: a link opens on it, and when the agent works on a
page there (navigates it, clicks or types in it, or opens a tab), the
stage shows that page's tab. The chats that work in one folder, as the
chats of a project, or of a sketch (see below), share its stage, which
shows beside whichever of them is shown. Each tab is the
human's, as every tab they open is, or of one of the chats, as every
tab its agent opens is. Every chat's agent sees every page of the stage,
the human's and the other chats' too, and judges which are of its
work, as Cox's own `cox.md` tells it; but a page another chat's agent
is at work on, one it last worked on, or else opened, or was handed,
while that chat takes a turn, the agent reads and does not act on:
what clicks, types, goes to another address, answers a dialog or
closes the page is refused, the refusal naming the chat, until its
turn ends. The
menu of the right button on a tab of the human's hands it to the chat
shown, "Hand to" and its name, which makes it that chat's, as on the
page the human has open; on a chat's, "Take back" makes it the
human's again, which no agent is at work on; and on
another chat's, "Show" and its name shows that chat. A chat archived
gives its tabs back to the human. The tabs of the chats not shown are
dimmer, and what the agent of a chat not shown opens there opens
behind the tab shown, the stage staying as it was, until that chat is
shown, which shows its last tab. A stage is kept with the chat each tab
is of. The
bar along its top keeps the views of the project in sight first, side
by side: "Activity" (see below), "Map", when the project has a map, "Files",
"Changes", when the project is in a Git repository, and the other
plugins' pages, such as "Knowledge" and "Triggers", each
a tab that the bar shows by its button, not among the
others, and kept there while its tab is open, each button by its icon,
named where the mouse rests, but that of the view shown, which names
it. The other tabs are in a column along the stage's left, under the
bar, one under another in the order they were opened, each with an icon of what it shows, a page, a
Markdown file, code, a terminal, the flow of a tool call, or, for a
page, the icon of its site, as a browser's tab shows it, the first of
those the page names, or the site's `/favicon.ico`, that loads, the
last page's staying while the next loads, and its
title, and, where the mouse rests or for the tab shown, the × that
closes it; a tab an agent is at work on, as its chat takes a turn, has
the icon of what it shows, a page's too, in a color of the chat's own, one of eight of the accent's
lightness, the first chat's the accent, and a ring of it beside its
title, which names the chat where the mouse rests, and is not dimmer
for being another chat's. The ‹ at the column's foot folds it to the tabs' icons,
each named where the mouse rests, and the › unfolds it, for every stage at once. The +
at the bar's end opens a menu of what the human opens there: a line at
its top, where Enter opens what they typed, a file of that path, from
the chat's folder, if there is one or the line starts as a path does,
or else a page, when the line has no space and what comes before its
first `/` has a scheme, or a dot or a port, as a host has, or else that
file again, when the line has no space and a `/`, as `src/new.rs`, or
else a command, which runs in a terminal; then "Web page", a
new tab, "Terminal" (see below), "File…", which asks for a file
and opens it as a link to it does, "Android devices", the virtual
devices of the Android emulator, and, on macOS, "Mac windows", the
windows of the Mac's other apps (see below). Escape closes the menu. Their keys, Command-T, Control-` and
Command-P (Control-T and Control-P off macOS), open them from anywhere
in the window, on the stage of the chat shown. The cursor over a page is the one
the page asks for, as a pointing hand over a link, or an I-beam over
text and fields, and the page has the keyboard just while the stage
has it, as after a click on the page, so that the field it is in shows
its caret and focus ring then, and not while the human types in the
chat or the window is behind another. A page that asks for a window of its
own, as a link with `target="_blank"` does, gets a tab instead. A link from the
conversation gets a tab of its own, unless a tab shows it already. A link
in a page to a file, as `file:///home/me/app/src/main.rs#L12`, opens the
file on a tab as a link from the conversation does, at the lines it
names, unless the file is an HTML page, which the page goes to as any
other; so a page the agent makes, such as a diagram of the project, can
lead to the code it draws.

"Activity", a plugin (see below), at `cox://activity/`, shows the human
what goes on in the stage's folder, in two tiers: first what waits for
them, the chats of the folder, and of its sketches, that ask them
something, or whose turn ended while they were not looking, with the
goal each works to, a click on one showing the chat; then what
happened without them, the last first: the schedules that ran there
(see [Schedules](#schedules)), the pages of the wiki written from there,
the human's from its chats and the project's own, and the sources kept
there (see [Research](#research)), each of which a click opens on
"Knowledge" or "Triggers". The home's tells of every folder: the
chats that wait in any project, each with its project, the schedules
that ran anywhere, the pages of the human's wiki written from any chat,
and every source. Its button on the bar counts the chats
that wait, and the page reads it all again every two seconds.

A tab can show a Markdown file too, drawn as the conversation draws the
agent's replies, for the human to read: the agent opens it with Cox's
`show_markdown` tool. The tab follows the file as it changes, and the
agent showing the same file again brings its tab to the front.

A tab can show a file as it is written too, such as code, with the
numbers of its lines, for the human to read: the agent opens it with
Cox's `show_file` tool, at the lines it talks about, which the tab marks
and scrolls to. A click on a path in the conversation, as the agent
writes `src/main.rs:12`, opens the file the same way, or as Markdown when
it is a Markdown file and the path names no lines. The tab draws only the
lines in sight, so a long file scrolls as a short one does, and follows
the file as it changes. It draws the code in colors, as its syntax says,
through [tree-sitter](https://crates.io/crates/tree-sitter)'s grammars
for Rust, TOML, Markdown, JSON, YAML, shell scripts, JavaScript,
TypeScript and Python, with the code within the code too, as a Markdown
code block's or a Rust macro's. It parses the file in the background,
again only where it changed, keeping the colors it had until then.

A tab of code asks the language server of its language what the code
means, through the Language Server Protocol, for any language a server
knows, whether Cox draws its colors or not, as Go. Cox knows a server for
some languages, which has to be on `PATH`: `rust-analyzer` for Rust
(`rustup component add rust-analyzer`), `pyright-langserver` for Python,
`typescript-language-server` for TypeScript and JavaScript, `gopls` for
Go, and `clangd` for C and C++. A project names servers of its own in
`.cox/servers.json`, ahead of Cox's, as Cox's own
[`servers.json`](crates/cox-editor/src/lsp/servers.json) names them:
each by its name, with its `command`, the `languages` it knows, as the
protocol names them (`rust`, `python`, `typescriptreact`, ...), the
`markers`, files a folder that is a project of them has, its
`initializationOptions`, and its `settings`, by section, as the server
asks for them. A server of Cox's the project names takes what the
project gives of it in place of Cox's, as `"rust-analyzer": { "settings":
... }`, and named as `null`, it is not started. Each project gets one
server of a language, started when a tab, or the map, first needs it and
shared by the tabs of all its chats, until Cox quits; Cox's
`rust-analyzer` builds what it needs in `target/rust-analyzer`, so that
it does not hold up the project's builds. Cox asks a server only what it
says it answers, and says so of the rest. The mouse resting on a
name shows what the server says of it, its type and documentation. A
click on a name with Command held, Control off macOS, goes to where it
is defined, as "Go to definition" in the menu of the right button does;
"Find references" there lists, under the code, where the name is used,
to go to each in turn. Going to a place in another file, the tab shows
that file: the ‹ and › beside its path go back and forward between the
places it went to, as Command-[ and Command-] do (Alt-Left and
Alt-Right, and Control off macOS), and the mouse's buttons for that.

"Files" opens a tab of the project's files as a tree,
without what Git ignores, for the human to open one: a click on a
folder opens or closes it, and a click on a file opens it, as a link to
it does. The tab follows the folders open in it as files come and go,
and a closed chat's stage opens it again with the same folders open.

"Changes" opens a tab of what changed in the project's files,
as Git tells it, file by file and line by line, with the files Git does
not track yet but does not ignore: in a sketch, all it changed since it
started, what it committed too, and elsewhere what changed since the
last commit. Each file is a card, headed by how it changed, its path
and the lines it gained and lost, over its lines, drawn in the colors
of their syntax as a tab of code draws them; of a line the change put
in place of one it removed, the words that differ are marked on both,
a line paired with the first one after it that has enough in common.
A click on a file, or on a line, opens the file there, as
a link to it does. The tab follows the files as they change, and while
the stage is open, its button counts the files that changed, whether
the tab is open or not. The agent
opens it with Cox's `show_changes` tool, to show the human what it
made, and hears which files changed; given another folder, such as that
of a sketch it opened, the tab shows what changed there. A sketch marks
where it starts with a ref of its worktree's own,
`refs/worktree/cox-start`: the commit it starts from, or the empty tree
for a sketch from nothing, which then counts all it made; a sketch opened
as a project forgets it.

"Map" opens a tab of the project's maps, for the human to see
where things are before they read them: not the files, which "Files"
lists, but what the parts of the project mean, in tiers, which an agent
writes. A tier is rows of boxes; each box of a tier opens a tier of its
own, down to boxes that hold paths of the code; and boxes of other
tiers can stay around the tier that is open, above and below it, so
that the lines out of it reach them, and a tier reads from the top down
as the top tier does. What the tiers are, and what goes in each, is the
project's to decide, in its chats and its `COX.md`; Cox only draws the
file. Each box has a line on what it is, for a reader
who does not know the project, how many of it there are, and the paths
it holds. A click on a box opens the tier inside it, or, for a box that
holds paths, the box on its own, with its files, each of which a click
opens as a link to it does, and under each file what the project's
language server says of it, once it has read the project: the symbols
the file defines, as the server lists them, each opening
the file at its line, how many places use each, and which files use
them; until the server has answered, the file says what the server is
doing. The path above the boxes leads back up, and over the boxes a
line of buttons names the ways out: back to where the tab was, as a
code tab's ‹ does, naming the place and the step chosen there
(Backspace, Alt-← and Command-[ too); up a tier, naming it (the ↑ key
too, and a click on the name of the box the tier is the inside of,
marked ↑, over its boxes); and forward again (Alt-→ and Command-]). So
a drill down and the way back are each a click, over and over. A map
can have flows
too, the orders in which the boxes talk, each a list of steps from one
box to another, written between the boxes that hold the code: "flows"
above the boxes lists them, and a flow chosen is drawn as a sequence
diagram of the tier that is open: the boxes its steps go between, rolled
up to the tier's, as columns, each with a lifeline, in the order of
the map: the top tier's boxes as it draws them, each box that opens a
tier on the way down to the tier open replaced by that tier's boxes,
in its order, in a frame named with the box, so that the frames nest
down to the tier open, whose frame is drawn full and the others
dashed, and a drill down opens a box in place, the other columns
staying where they were; since every box of the map is in a tier that
some box of the tiers above opens, the boxes of those tiers that hold
the rest of the flow are columns too, as a flow that comes into the
stage through the chat shows the chat as a column, named with the tier
it is in, so that the whole flow is read from any tier, and the
frame's boxes among them; the name of each frame goes up to the tier
that shows its box, and stays in sight as the diagram scrolls; and the
steps as rows from the top, an arrow from the
box that talks to the one it talks to, with what the step says; a step
back, the answer to an earlier step, which the file marks `back`, is
dashed, and a bar on a box's lifeline shows it at work, from a step
into it until its step back; a step
inside a box is a loop on its lifeline, and a click on it drills down
to the step, at the first tier inside the box whose boxes the step goes
between, past the tiers where it is still inside one box, or, when it
stays inside a box that holds paths, to that box on its own, where it
goes between the functions it runs (see below); so a flow is read from
the top down as the map is, and a column opens its box as a card does. A step rolled up to the tier's boxes says
what it is in words, and drawn between the boxes it is written between,
as deep as the map goes for it, is named by the function it runs, which
Cox asks the project's language server for, at the place the step is
made, its `at`: the function the line is in, and what it calls on that
line, as the server's call hierarchy has them, or, for a server with
none, as it says each name called on the line is defined, the names
called being those the grammar's tags mark as calls, for a language
Cox draws, or else those the server's semantic tokens mark as
functions, methods or macros, of which the diagram
names the one in the box the step goes to, or else the last of the
project's own called on the line; the name opens the function on a tab,
and the line under the flows names them all. A line that calls nothing,
as when the code moved, says so, for an agent to bring the map up to
date. A box that holds paths, opened from a column of the diagram, or
from its card, while a flow is chosen, draws the flow one step deeper,
over its files: the box itself as the functions of its code the flow
runs, as the server says of the places its steps are made, a step
into the box arriving at the function its line calls in the box, a
step out of it leaving the function its line is in, and a step inside
it going from the one to the other, each a column that opens the
function on a tab, between the other boxes, those of its tier and of
the tiers above; and the files' symbols below mark those the flow
runs. So a flow is drilled down to the functions that make it, and a
step whose place the server has not told stays on the box itself.
The mouse resting on a step shows the code it is made at: the line
its `at` names and a few around it, colored as a code tab draws them,
under the lines that open what the line is within, as its function's
`fn` line and its impl's, dimmer, as the syntax has them, with the
function the line is in and what it calls, as the server says, so that
the name the step bears is read beside the line it comes from; a click
on a line of it opens the file there on a tab.
"on the map" draws the flow on the tier
instead, its steps numbered on the lines between the boxes, with the
step chosen lit and the other lines dimmed. Either way the ← and →
keys go from step to step, and the step chosen stays chosen through a
drill down and back up.
A project can have several maps, each a picture of it from a top of
its own, as one of what it is made of and one of how it is deployed:
each is a file in the project's folder, in `.cox/maps`, named after the
map, as `.cox/maps/parts.json` for the map `parts`, with, at its top, a
line on what it shows as against the others (`says`), listed by name.
A map is written by
an agent once and kept up to date by the agents that change the code,
so that it is there whenever the tab is opened, the same whatever
changed; a project tracks them or not as it likes, and Cox's own
repository does not, so a clone of Cox gets its maps written where it
is. With more than one, the tab names them over the map shown, a click
on a name opening that map at its top, and the line on what it shows
where the mouse rests. Over a map Cox lays what it knows: what changed, as
"Changes" shows it, falls on the boxes that hold the files, and up on
the boxes that open their tiers, with how many lines changed in how
many files, so that the human sees where a change falls in the project
before they read it; and a box whose paths
the project no longer has is marked stale, for an agent to bring the
map up to date. A project with no map has no "Map" on the bar, and an
agent asked to show the map gets a tab that says so, and is told how
the file is written. The agent opens
a map with Cox's `show_map` tool, by its name, or the first unless it
names one, at a tier, or a box in it, to show
the human where what it talks of is before it talks it through, and
hears what the map shows there, box by box, and the names of the
project's other maps. The tab follows the files
and the project as they change, and a closed chat's stage opens it
again where it was open. The map is a page of the stage, one of its
plugins (see below), at `cox://map/`, where the map is open being the
page's hash, as `cox://map/#cox/stage`, after the map's name and `:` for
another than the first, as `cox://map/#deploy:hosts`, so that the
page's ‹ goes back from one map to the one before.

A tab can show a plugin: a page, in HTML and JavaScript, that draws
what Cox knows as a tab Cox draws would, for whoever wants a view of
their own on the stage, the agent included, since it writes HTML and
reads pages. "Activity", "Map", "Knowledge" and "Triggers" are built in. A plugin is a
folder with an `index.html`, and a `plugin.json` naming its `title` if
it has one: those built into Cox, in `crates/cox/plugins`, and the
project's, in `.cox/plugins/<name>` under its folder. The bar has a
button for each, "Activity" and "Map" first and the rest after "Changes", which opens the plugin's page at
`cox://<name>/`, an address the stage serves itself, through CEF, from
the plugin's files, or shows the tab that has it, wherever the page has
gone; the agent opens it as any page. A page at a `cox:` address is
drawn as a tab Cox draws, without the toolbar a page of the web has,
since the page is the tab. A plugin reads what Cox knows as JSON from
`cox://api/<what>`: `cox://api/map/<name>` is the map of the project
named `<name>`, as its file in `.cox/maps` has it, and `cox://api/map`
its first, with the names of all its maps, and what Cox knows of each
box laid over: the files it holds, as
the project has them now, what changed in them, and which are gone; and
`cox://api/file/<path>` is what the project's language server says of
the file at `<path>`, an absolute path in the project, once it has
answered, which the page reads again for: the symbols it defines, how
many places use each, and which files use them;
`cox://api/call/<path>:<line>` is what the server says a line of the
file calls, for a step of a flow made there;
`cox://api/wiki` is the human's wiki, its pages, each with what it
links to, what links to it, where it was used and what Cox marks of it,
and `cox://api/wiki/<page>` a page, with its text as HTML;
`cox://api/project-wiki` and `cox://api/project-wiki/<page>` are the
project's wiki, the same way, and whether the folder is the home;
`cox://api/raw` is the sources, each with where it was read, when and
by which chat, and `cox://api/raw/<name>` a source, as a page is told;
`cox://api/triggers` is the triggers of the folder, as "Triggers" shows
them, those on a schedule (see [Schedules](#schedules)) with when they
run next, and `cox://api/triggers/set/` and a change, as JSON, turns one on
or off, `{"folder": ..., "file": ..., "on": false}`, a schedule's in the folder
where its chat works, or orders them,
`{"folder": ..., "order": [...]}`, answering them so;
`cox://api/triggers/save/` and a card, `{"folder": ..., "file": ...,
"draft": {...}}`, writes the trigger as the card says, answering them
with the file it `saved`, or the `error` why not; and
`cox://api/triggers/discuss/` and a line, `try/` and a card,
`discussion/` and `tried/` and the card's id, and `end/` and the id, talk
a card through, try it, say how those stand, and end them;
`cox://api/activity` is what goes on in it, what waits for the human
and what happened, as "Activity" shows it; and
`cox://api/lines/<path>:<line>` is the lines of the file around that
line, each as its stretches with what the syntax says each is, and
before them the lines that open what the line is within, as its
function's and its impl's, for a page to show the code a step is made
at, colored as the code tabs color it. It
opens a file on a tab
by going to `cox://open/<path>#L<line>`, which the stage takes as it
takes a link to a file (a page that is not itself a file may not link
to one), and shows a chat by going to `cox://chat/<key>`, by the
number `cox://api/activity` tells it by. A plugin draws in the window's
colors by linking `cox://api/theme.css`, whose custom properties are
the colors Cox draws its own tabs in, as `--surface`, `--accent` or
`--asking-ground`, as the built-in plugins do. The Map plugin draws its own cards and lines, with no
library, and reads its JSON again every two seconds, as Cox's own tabs
follow what they show.

A tab can be a terminal too, through
[alacritty_terminal](https://crates.io/crates/alacritty_terminal).
"Terminal" in the menu of the stage's + opens one with the human's
shell, working in the chat's folder. The agent runs a command in one with Cox's `run_command`
tool: the stage shows it running, the human can type into it, as to
answer what it asks, and the agent gets what the terminal shows and the
exit code once it ends. A command gets the tab of the agent's own last
one that ended, or a new tab. When the agent stops waiting for it, as when
the human stops the turn, the command gets Control-C. Copying and
pasting in a terminal is Command-C and Command-V on macOS, and
Control-Shift-C and Control-Shift-V elsewhere, where Control-N,
Control-O, Control-T, Control-P and Control-\\ go to the terminal too.

A tab can be an Android emulator too, as Android Studio's "Running
Devices" shows one, for the human to try an app they make for Android
on the stage, beside the chat. "Android devices", in the menu of the
stage's +, opens a tab of the virtual devices, as Android Studio's
Device Manager lists them: each with the name the human gave it, the
phone it is made as, the Android API it runs, the size of its display,
and whether it runs. Cox reads them from the folder the SDK's tools
keep them in, `ANDROID_AVD_HOME`, or else `avd` in
`ANDROID_EMULATOR_HOME` or `ANDROID_USER_HOME`, or else
`~/.android/avd`, and reads it again every two seconds, so a device
Android Studio makes shows as it is made. Making a device, changing
it, deleting it, and the system images it runs, are Android Studio's.
A device that runs has "Show", which shows its tab, and "Stop"; one
that does not has "Start", "Cold boot", which starts it afresh rather
than from the state it saved, and "Wipe data", which takes its apps
and data away first, once the human says yes. Each opens the device on
a tab of its own, or shows the tab it has. The tab starts the emulator
of that device, with no window of its own (`emulator -avd <device>
-grpc <port> -qt-hide-window`, with `-no-snapshot-load` for a cold
boot, and `-wipe-data`), or reaches it if it runs already, by the file
each emulator writes of itself as it starts
(`avd/running/pid_<pid>.ini`, in `~/Library/Caches/TemporaryItems` on
macOS and `$XDG_RUNTIME_DIR` on Linux, with the token its gRPC service
asks for, if it asks for one). The tab draws the emulator's display,
as big as fits, which the emulator's gRPC service streams at the size
the tab draws it, and sends it, over the same service, the human's
mouse as a finger, a click as a tap and a drag as a swipe, the wheel,
and the keys typed, Escape as Back; the bar over the display has Back,
Home and Overview, and Stop. Cox finds the emulator in the SDK that
`ANDROID_HOME` names, or `ANDROID_SDK_ROOT`, or else where Android
Studio puts it (`~/Library/Android/sdk` on macOS, `~/Android/Sdk` on
Linux), as the human's shell, which Cox takes its environment from on
macOS, sets them; with none, the devices' tab says so. The emulator
Cox started stops as its tab closes, or Stop is clicked, or Cox quits,
saving its state as it does when its window closes; one that ran
already goes on. A closed chat's stage, or the next run's, opens the
tab again, reaching the emulator if it runs, and else with "Start" to
start it.

The agent tries its app on a device from the tab too, as the human
watches: Cox's tools list the devices (`android_devices`), show one on
its tab, a new one of the chat's, starting its emulator if it does not
run, and answer a picture of its display (`android_look`), and tap,
swipe, type and press a key on it (`android_tap`, `android_swipe`,
`android_type`, `android_key`), over the same gRPC service, answering a
picture of the display after, 1280 pixels along its longer side, in
whose pixels the agent says where it acts. A look waits for the display
while the emulator starts, and the display comes, until the tab is
drawn, at a size of its own, so that the agent sees it whether the tab
is in sight or not. The tab draws the agent's pointer where it taps and
swipes, as a page's does, so that the human sees what the agent does,
as it does over a window of the Mac's (see below).

On macOS, a tab can show a window of another app too, for the human to
see, beside the chat, the app the agent makes and tries out, rather than
have it tried behind their back. "Mac windows", in the menu of the
stage's +, opens a tab of the other apps' windows on screen, each by its
app and its title, which ScreenCaptureKit lists: "Show" shows the window
on a tab of its own, or the tab that shows it, and "Bring to front"
brings the window itself up. The tab draws the window as big as fits, as
ScreenCaptureKit streams it at the size the tab draws it, and only draws
it for the human: to use the app, they bring the window itself to the
front, with the tab's "Bring to front" too. The agent uses it from the
tab, as the human watches: Cox's tools list the windows
(`mac_windows`), show one on a tab of the chat's and answer a picture
of it (`mac_window_look`), and click, type, press a key and scroll in it
(`mac_window_click`, `mac_window_type`, `mac_window_key`,
`mac_window_scroll`), posting each to the window's app while the window
stays behind Cox, and answering a picture of it after, at most 1568
pixels each way, in whose pixels the agent says where it acts. The tab
draws the agent's pointer where it acts, as a page's does, so that the
human sees what the agent sees and does. An app behind another takes
the mouse and keys posted to it only in part, as a page in Chrome takes
a click only while Chrome takes itself for the active app: from the
agent's first click until its turn ends, the window's app takes itself
for the active one, while the app in front stays in front, through
functions macOS keeps private, which it may change or drop. macOS asks
the human to allow Cox to record the screen, for the pictures and the
windows' titles, which the tabs ask for with "Allow…", and only then,
since the system asks again each time Cox looks for the windows while
it may not; and to control the computer, for what the agent posts,
which the agent's first act asks for, the tool saying so until the
human allows it. A closed
chat's stage, or the next run's, opens the tab again on the window of
the same app and title, if one is open, and else with "Show" to look for
it again.

Cox hands each chat's agent its own tools as an MCP server, which is
`cox` itself run as `cox --mcp <socket>`: the agent starts it, and it
carries what the agent says to the chat on a Unix socket of the chat's
own, which Cox answers. An agent reaches only its own chat that way.
The sockets are in a folder of coxd's run among the system's temporary
files, `cox-` and its process id, which only the human may enter; the
window keeps one of its own so, for what else it keeps only while it
runs, as a throwaway profile of the stage. A Cox that crashes leaves
its folder, which the next Cox to start takes away.

CEF opens the Chrome DevTools Protocol on a port that is free when Cox
starts, so two Cox can run side by side, and Cox hands each chat's
agent the
[Chrome DevTools MCP server](https://www.npmjs.com/package/chrome-devtools-mcp)
connected to it, so the agent reads and drives the pages the human sees
in that chat. The server runs through `npx`, so Node has to be on
`PATH`; without it, the agent gets no tools for the stage. The server
connects through a relay in Cox, one for each chat, which passes
everything on but keeps the agent to the pages of its chat's stage: the
agent hears nothing of other stages' pages, nor of the rest of the
browser, and the pages it opens are tabs of its chat. Of a page another
chat's agent is at work on, the relay refuses what acts on it, as
`Input.*`, `Page.navigate` or `Target.closeTarget`, and answers
`Page.bringToFront`, with which the agent's tools choose a page to
read it too, without passing it on, so that the stage stays as it is;
a script the agent runs on the page is not told apart from those its
tools read the page with, and is not refused. The relay also tells the stage where the agent
points, clicks and types, so the stage can draw the agent's pointer over
the page. The server's tools that read or write files, as a screenshot
saved to a file, may do so in the chat's folder and in `/tmp`, where
Claude Code keeps its scratchpad, beside the system's folder of
temporary files, which on macOS is elsewhere.

Cox keeps what lasts across runs in a data folder, one for each place
Cox runs from. The Cox that `cargo xtask install` puts in place keeps it
in `~/Library/Application Support/Cox` on macOS and `~/.local/share/cox`
on Linux; any other, such as `target/debug/Cox.app`, keeps its own in
`cox-data` beside it, as `target/debug/cox-data`. A Cox given
`--data <dir>` keeps it in `<dir>` instead, to run apart from what it
kept before, or on a copy of what another Cox kept. In the folder, each
kind is kept apart
([`storage.rs`](crates/cox/src/storage.rs) lists them):

- at its top, what the human writes or chooses: `settings.json`,
  `agents.json`; beside them `sketches`, the folders the sketches work
  in, and `logs`;
- in `state`, what Cox would lose: the chats, in a SQLite database,
  `chats.sqlite`, the stage's profile, `stage`, when each schedule
  last ran, `schedules.json`, which triggers are off and their order,
  `triggers.json`, how each last ran, `trigger-runs.json`, what a chat
  of each folder said last as its turn ended, `last-turns.json`, the
  pictures of the regions handed to the chats, `pictures`, and the
  icons the human gave the projects, `icons`;
- in `cache`, what Cox can make again: `registry.json`, the agents it
  downloads, in `agents`, and `translations`;
- in `run`, what is there only while Cox runs: `coxd.sock` and
  `inspect.sock`.

The stage keeps its cookies, logins and storage in its profile, which
lasts across runs. Only one Cox at a time can use a profile; a second
Cox from the same place gets a profile for that run only.

On Linux, `cargo run` is enough: CEF is linked into `cox`, and its helper
processes run `cox` again.

On macOS, CEF is a framework that the app loads when it starts, and its
helper processes are small apps of their own, so Cox has to run from an
app bundle. The cef crate's build also compiles a C++ wrapper with CMake,
which needs `cmake` and `ninja` (`brew install cmake ninja`).

```sh
cargo xtask bundle            # or: cargo xtask bundle --release
target/debug/Cox.app/Contents/MacOS/Cox <agent command>
```

`cargo xtask bundle` builds `cox` and `cox-helper`, then lays them out as
`Cox.app` next to them: `cox` as `Contents/MacOS/Cox`, the framework and
five copies of `cox-helper` (`Cox Helper.app`, `Cox Helper (GPU).app`,
...) under `Contents/Frameworks`. CEF fixes that layout and those names,
starting from the executable's. The app's icon,
[`app-icon.svg`](crates/cox/assets/app-icon.svg), it renders into
`Contents/Resources/Cox.icns`, at each size macOS asks for. It then signs the app as a whole, which macOS asks of an app before it shows the app's
notifications: with the certificate `COX_SIGNING_IDENTITY` names, or
else "Cox Local Signing" when the keychain has it, or else ad hoc. macOS
keeps what the human allowed Cox, as to record the screen, for the app
it knows by its signature, which for an app signed ad hoc is the
build's own hash: each build of it is another app to macOS, which asks
again. Signed with a certificate, every build is the same app. A
certificate of one's own is enough, made once in the login keychain
with Keychain Access, "Certificate Assistant", "Create a Certificate…":
named "Cox Local Signing", "Self Signed Root", of the type "Code
Signing". The first build to sign with it asks for the login keychain's
password, for `codesign` to use the certificate's key, once for each
part of the app it signs: "Always Allow" lets it from then on. `cargo run` on macOS stops at once,
saying where the framework was expected.

## coxd

The chats run in coxd, `cox [--data <dir>] --daemon`, a process of
their own apart from the window: it opens the chats kept, starts their
agents, and runs them, whether a window shows them or not. The window
reaches it on a Unix socket, `run/coxd.sock` in Cox's data folder (see
[The stage](#the-stage)), one message a line, each a JSON object, as
[`wire.rs`](crates/cox/src/daemon/wire.rs) says them, and shows what
coxd tells it. A window that finds no coxd there starts one, of the
same program and the same data folder, in a process group of its own,
so that a Control-C in the terminal the window started from leaves it.
`--home` and the `<agent command>` of the window go to coxd as it comes,
for the chats it opens from then on.

So that the chats, and the schedules (see [Schedules](#schedules)), run
from the time the human logs in, without a window opening first,
`cargo xtask install` has the installed Cox's coxd start at login: on
macOS, a launch agent, `~/Library/LaunchAgents/com.github.tennashi.cox.coxd.plist`,
which launchd loads at once too, starting coxd unless one runs already;
on Linux, a unit of systemd's user manager,
`~/.config/systemd/user/coxd.service`, which starts at the next login.
Neither starts coxd again once it stops: a window that finds none
starts one, as it does without them. A coxd launchd starts takes in the
environment of the human's login shell, as a Cox the Finder opens does
(see [The stage](#the-stage)). To have it start at login no more,
`launchctl bootout gui/$(id -u)/com.github.tennashi.cox.coxd` and take
the file away, or `systemctl --user disable coxd.service`.

coxd takes one window at a time: a second Cox from the same place says
another window has the chats, and stops. The window closing leaves
coxd, and the agents at work, running; the next window to open shows
the chats as they are then. While no window is there, coxd runs a
command an agent asks for, or a goal's, itself, on a terminal of its
own that no tab shows, as for a chat on a schedule (see
[Schedules](#schedules)): the agent gets what the terminal shows and
the exit code, as from a tab, and a command it stops waiting for gets
Control-C, and then, if it goes on for five seconds more, a hang-up. A
command that asks the human, as a login does, fails; what else a chat
asks of its stage, as to show a file, shows nowhere; and an agent that
starts gets no tools for the stage. coxd follows `settings.json` and
`agents.json` as the window does (see [The stage](#the-stage)), and
keeps a log of its own run beside the window's (see
[What Cox keeps of each run](#what-cox-keeps-of-each-run)).

coxd runs until it is stopped, with SIGTERM, SIGINT or SIGHUP, as
`pkill -f 'cox --daemon'`, which stops the chats' agents, or until its
socket is taken away, as with its data folder. When its program was
replaced since it started, as `cargo xtask install` replaces it, and
no chat is at work, the next window to come has it stop, and starts the
new one; while a chat is at work, it goes on as it was, until a window
comes once none is.

## Schedules

The human has an agent run on a schedule, with no human there: a chat
that opens at the times they say and goes on its first line, as to find
out every morning what changed in what they follow. Each is a trigger
(see [The stage](#the-stage)) that runs `on` a `schedule`, `by` a `new`
chat, of their workflow, in `triggers`, or of a project, in
`.cox/triggers` in its folder, whose properties say when it runs, where
its chat works and what its agent does unasked, and whose rest is the
chat's first line:

```markdown
---
on:
  schedule:
    cron: "0 9 * * *"
by: new
folder: ~/notes
allow: [fetch:*, search:*]
---

Find out what changed in Rust since {last}.
```

`cron` is the times it runs, as cron writes them, in the time of the
place Cox runs in: the minute, the hour, the day of the month, the
month and the day of the week, each `*`, a number, a range, as `1-5`,
any of them with a step, as `*/6`, or a list of those, as `1,3`, months
and days named by their first three letters too; `0 9 * * 1-5` is nine
on each weekday, and `0 */6 * * *` every six hours, from midnight.
`on` also takes it on one line, as `{schedule: {cron: "0 9 * * *"}}`.
`folder`, whole or from `~`, is where the chat works, in the project of
that folder, which Cox makes if it is not there; with none, the chat
works in the home, as Cox's own, and a project's works in the project.
`allow` and `reject` list the patterns of the permission requests Cox
answers in the human's place, as `open_chat`'s take them (see
[The stage](#the-stage)); the rest the agent asks the human, as any
chat does. In the first line, `{name}` stands for the trigger's name,
`{now}` for when it runs, and `{last}` for when it ran last, or
`never`. A schedule runs on no other event, and no shell runs it.

Every minute, coxd (see [coxd](#coxd)) runs each schedule that is due
and turned on where it works, whether a window is there or not: one
that never ran at once, and one a run of which Cox missed, as while the
machine slept, once, however many it missed; while the chat of its
last run is at work, it waits. Cox runs those of each project it has
opened. A run is a chat named after the trigger and when it ran, as
`rust-news 11/06 09:00`, whose first line goes in the human's place.
When each last ran is kept in `state/schedules.json` in Cox's data
folder, by its file's path. One Cox cannot
read is said in coxd's log, and does not run.

"Triggers" on the stage's bar lists them under "Schedule", in the
folder where their chats work, and in the home, those whose folder is
no project's: each with how it runs in words, as `Daily 09:00`, where
its chat works when that is elsewhere, when it ran last and when it
runs next, or that it is due, and a switch that turns it on or off;
its card says its cron, where its chat works, and what its agent does
unasked.

## Research

What a chat's agent reads, as a web page, a paper, a release note or a
command's output, it keeps as it read it, with Cox's `raw_keep` tool,
as the material of research the human has it do, apart from what is
made of it, as a report or a page of the wiki, which names the sources
it was made from, so that it can be checked against them, or made
again; whether it becomes a page of the wiki is for whoever writes the
page. The sources are Markdown files in the folder `settings.json`
names, whole or from `~`, as `{"raw": "~/notes/raw"}`, or else `raw` in
the home the settings set, as `~/cox/raw`, by topic, the research they
are of, each named after the day it was kept and its title, as
`rust-news/2026-11-06-announcing-rust-1-92-0.md`, with, in its
properties, each apart, for Obsidian to sort them by, where it was
read (`source`), the day (`kept`), the chat that kept it, by its name
and its number (`chat`, `chat_id`), and its project, if it is in one
(`project`). A
source is never written again: what changed is another source, kept
beside it. `raw_read` reads a source, or lists those of a topic, the
last kept first, and `raw_search` finds those that have every word it
looks for. "Knowledge" on the stage's bar shows them to the human,
under "Sources" (see [The stage](#the-stage)).

A web page the agent keeps by its URL alone, and Cox reads it itself,
whole, since an agent's own fetch tool may answer what a smaller model
made of the page rather than the page, as Claude Code's `WebFetch` does:
its article, apart from the site's menus, as Readability finds it, in
Markdown, as Obsidian's Web Clipper reads it, which Cox keeps, and
answers the agent. The page is anyone's, so Cox reads it apart from
itself, as `cox --read-page <url>`, which it stops after a minute, since
a page can be made to keep a parser busy for longer; and reads only
`http` and `https`, of no address of the machine's own or of its
networks', as `localhost`, `192.168.1.1` or a cloud's metadata, whether
named so or resolved to it, nor through a redirect, at most 5 MB of it,
and only HTML and text. A page its scripts draw, which Cox does not
run, or one past a login, the agent reads on the stage, with the
browser's tools, and keeps as text. What a source says goes to the
agent, from `raw_keep` and `raw_read`, between two lines that mark it
as the source's and not the human's, 30,000 bytes of it at a time,
`raw_read` going on from the line the last left off at. A source runs
nothing in an Obsidian vault: Dataview's ```` ```dataviewjs ```` blocks
are kept as ```` ```inert-dataviewjs ````, and a zero-width space breaks
every `$=`, Dataview's inline JavaScript, and `<%`, Templater's
commands, which it runs in a note made in a folder it watches.

So research on a schedule is a schedule (see [Schedules](#schedules))
whose first line asks for it:

```markdown
---
on:
  schedule:
    cron: "0 9 * * *"
by: new
allow: [fetch:*, search:*]
---

Find out what changed in Rust since {last}. Keep each source you read
with raw_keep, under the topic `{name}`, then write what changed, with
the sources it comes from, to a page of the wiki, `research/{name}`.
```

## What Cox keeps of each run

Cox says what goes wrong on standard error, as `RUST_LOG` asks, and only
the errors unless it asks for more. It also keeps all it says, down to
`debug`, or as `COX_LOG` asks, in a file of each run's own, for an agent
that debugs Cox to read and search once the run is over too: one JSON
object a line, with the spans it was said in, such as the chat it was
said of. Among it is what Cox and each chat's agent send each other over
ACP, a line at a time, under `cox_acp::wire`, what the agent and the
server of Cox's own tools send each other over the chat's socket, under
`cox::agent::mcp::wire`, what a tool asks of the stage, under
`cox::agent::mcp::asks`, and the stage carrying it out, under
`cox::stage::asked`, and what the agent writes to its standard error,
under `cox_acp::stderr`. Each line names the file and line of Cox's code
it was said at. How long each answer at a `cox:` address took is there
too, under `cox_browser::scheme`, as a warning when it took 100 ms or
more; and so is each while the window's thread, which draws the window
and on macOS gives CEF its turns, was busy for 100 ms or more, under
`cox::busy`, with CEF's turn when that was it, so that what else was said
then shows what kept it. The files are in `logs`
in Cox's data folder (see [The stage](#the-stage)), as
`~/Library/Application Support/Cox/logs` for the installed Cox on macOS,
and `target/debug/cox-data/logs` for the Cox built there, each named
after the time its run started and its process id, as
`20261101T160000Z-4242.jsonl`. Cox keeps the files of the last twenty
runs. A panic is kept there too, with its backtrace.

## A fake agent

`cox-fake-agent <scenario>` is an agent to run Cox with when no model
should answer: to try Cox out, to see a change working, or to make a
fault happen again. It speaks ACP as `claude-agent-acp` does, and answers
each line sent to it with a turn of the scenario, a JSON file, as
[`crates/cox-fake-agent/scenarios/tour.json`](crates/cox-fake-agent/scenarios/tour.json)
shows.

`cargo xtask run [--headless] [--data <dir>] [<scenario> | -- <agent command>...]`
builds Cox and runs it with the fake agent playing `<scenario>`, the tour
unless given, or with `<agent command>`, keeping what lasts in `<dir>`,
or afresh in `target/debug/cox-run` each time (see `--data` in
[The stage](#the-stage)), and says where the files of its runs are. Cox's
own chats work in `home` there, as its settings set it the
first time, until the human sets another.
`--headless`, on Linux, runs Cox on a display of its own, which Xvfb
draws in memory, with no screen; `cox --inspect` (see
[Asking Cox how it stands](#asking-cox-how-it-stands)) looks at it and
types into it there, through `xdotool` and ImageMagick's `import`:

```sh
sudo apt-get install xvfb mesa-vulkan-drivers xdotool imagemagick
cargo xtask run --headless
target/debug/cox --data target/debug/cox-run --inspect key ctrl-n
target/debug/cox --data target/debug/cox-run --inspect type 'hello'
target/debug/cox --data target/debug/cox-run --inspect key enter
target/debug/cox --data target/debug/cox-run --inspect state
target/debug/cox --data target/debug/cox-run --inspect screenshot shot.png
```

gpui draws with Vulkan, which `mesa-vulkan-drivers` gives a machine
with no GPU.

A scenario's `configOptions`, beside its `turns`, are the settings of
each session the agent opens, as ACP's `configOptions` says them, which
it sets as Cox asks; the tour has a mode, a model and an effort.
Its `commands` are the commands the agent runs, as ACP's
`availableCommands` says them, which it tells Cox of as each session
opens; the tour has three, and answers `/compact`. The agent takes
files whole, and pictures, and reads the files a line points to as
`claude-agent-acp` does, a link to each and its text, which a turn's
`when` can match: the tour answers a line that points to a file, and
draws an icon when asked to (see [The stage](#the-stage)).
Its `authMethods` are the ways to log in it offers, as ACP's
`initialize` says them: with them, the agent asks Cox to log in, before
it opens a session, or, with `"authRequired": "prompt"`, as it takes a
turn, until Cox logs in with a way the agent carries out, or the human
runs `cox-fake-agent <scenario> --log-in`, as a way of `"type":
"terminal"` with `"args": ["--log-in"]` has Cox run in a terminal, which
logs it in for the runs to come, until the file it writes among the
system's temporary files, in `cox-fake-agent-logins`, named after the
scenario's path, is taken away.

Each line takes the first turn not taken yet whose `when`, if it has one,
is in what the line says, as a goal's judge is told, and the turn takes
its steps one after another: `say`, `think`, `tool` and `tool_update`
(the fields of ACP's `tool_call` and `tool_call_update`, with
`toolCallId` as `id`), `update` (any of ACP's `session/update`, as it
is), `ask` (ACP's `session/request_permission`, with `then`, the steps
for each option chosen, by its `optionId`, or `cancelled`), `call` (a
tool of an MCP server Cox hands the agent, `cox` unless `server` says
another, with `arguments`, titled `title`, the tool's name unless
given), `sleep` (milliseconds), `stderr`, and
`fail`. In a step, `{cwd}` stands for the folder the session works in,
as Cox gave it, `{result}` for what the last call answered,
and a value that is just `"{result:number after as chat }"` for the
number after `as chat ` in it, as `open_chat` answers, to give
`wait_chat`. The turn ends with its `stop`, `end_turn` unless it says
another, or as `cancelled` when Cox cancels it.

`cox-fake-agent --from-log <run> [--chat <id>]` writes, as a scenario,
what a chat's agent did in a run Cox kept (see
[What Cox keeps of each run](#what-cox-keeps-of-each-run)), the chat
named by the number its span gives, or `judge` for a goal's judge: the
updates the agent sent, its permission requests, and what it wrote to
its standard error, turn by turn, with the waits between them, so that
what happened in the run happens again, as on another machine. The
agent's calls of Cox's own tools do not go over ACP, so the scenario
does not make them.

`cox-fake-emulator` is an Android emulator for where none runs, as on a
machine with no virtualization: it takes the arguments Cox starts
`emulator` with, writes the file of itself an emulator writes, and
serves the part of the emulator's gRPC service Cox uses, drawing a
phone's screen of its own, which a tap, a drag, the wheel, the keys and
Back, Home and Overview change. An SDK whose `emulator/emulator` is it
is one Cox finds as any other, and `cox-fake-emulator --make-avd`
writes a device for it, "Fake Phone", in the folder `ANDROID_AVD_HOME`
names, as Android Studio would:

```sh
cargo build --bin cox-fake-emulator
mkdir -p target/debug/fake-android-sdk/emulator
ln -sf ../../cox-fake-emulator target/debug/fake-android-sdk/emulator/emulator
export ANDROID_HOME=$PWD/target/debug/fake-android-sdk
export ANDROID_AVD_HOME=$PWD/target/debug/fake-android-avd
target/debug/cox-fake-emulator --make-avd
cargo xtask run --headless
```

## Asking Cox how it stands

Cox answers for its window to an agent that debugs it: one in a
terminal, one in a chat of another Cox, or one on a machine with no
screen. It opens a Unix socket, `run/inspect.sock` in Cox's data
folder (see [The stage](#the-stage)), as its window opens, and
answers on it how it stands, in words; with a screenshot; and to keys,
text, clicks and GPUI's actions, which reach the window whether it has
the screen's focus or not, as when the Cox under test runs behind the
Cox in use. The socket speaks MCP, as the server of each chat's tools
does, so `cox --mcp <socket>` reaches it as an MCP server, for an agent
set up with one, as in Claude Code's `.mcp.json`, once Cox runs.

`cox [--data <dir>] --inspect [<tool> [<arguments>...]]` asks it from a
shell: the Cox that keeps what lasts where this `cox` does, as
`target/debug/cox` for the Cox run from there, and the installed Cox
for itself, or where `--data` says, as `cargo xtask run` tells it
(`target/debug/cox --data target/debug/cox-run --inspect state`). It
prints what the tool answered; with no tool, it prints the tools. They
are:

- `state [<chat>]`: where Cox keeps what lasts and this run's log (see
  [What Cox keeps of each run](#what-cox-keeps-of-each-run)), the
  stages' DevTools port, the window and where the keyboard is, the chats
  in the order of the list, project by project, by the numbers they go
  by to the agents' tools, and one chat in full, the chat shown or chat `<chat>`: what it
  waits for, as its input shows while empty, its input, its stage's tabs, each with the chat it is of, and its conversation, entry by
  entry.
- `screenshot [<file>]`: the window as a PNG, written to `<file>`,
  `cox-screenshot.png` unless given: through ImageMagick's `import` on
  Linux, and `screencapture` on macOS, which has to be allowed to record
  the screen.
- `key <keystroke>...`: presses keys, as GPUI writes them, as `ctrl-n`,
  `enter`, `escape`, `1`, or `secondary-o` (cmd on macOS, ctrl
  elsewhere), and says whether each was handled.
- `type <text>`: types text, a character at a time, a newline pressing
  Enter.
- `click <x> <y> [left|right|middle] [<count>]`: clicks at a point of
  the window, in points from its top left corner, as the screenshot
  shows it: through `xdotool` on Linux, and not on macOS yet.
- `action <name>`: dispatches a GPUI action by name, as `cox::NewChat`,
  or `NewChat` when only one action's name ends so.
- `frames [<seconds>]`: where the window stuck over the last ten seconds,
  or `<seconds>`, up to a minute, for an agent to find when the human saw
  it stick, and read what the log said then: each while the window's
  thread was busy for 100 ms or more, with what, when Cox knows; and each
  page of the stages, by its address, with how many times the wheel
  scrolled it, how many pictures CEF painted of it, and each wait of
  50 ms or more from a turn of the wheel to the next picture, which the
  human sees as the page sticking, as when what carries the wheel and the
  asks for pictures to the page is busy. A page at its end, which the
  wheel cannot scroll further, paints nothing either.

## Using Cox to build Cox

The Cox in use runs from where `cargo xtask install` puts it, so that
building and trying a change in `target/` leaves it be:

- On macOS, `~/Applications/Cox.app`: run
  `~/Applications/Cox.app/Contents/MacOS/Cox <agent command>`, or open
  it as any app once `settings.json` names an agent of `agents.json`
  (see [The stage](#the-stage)).
- On Linux, `~/.local/bin/cox`, with what it needs of CEF in
  `~/.local/lib/cox`.

A Cox started from it, such as `target/debug/Cox.app`, runs beside it
with a DevTools port and a stage profile of its own (see
[The stage](#the-stage)), so each agent drives its own stage. Installing
again while the Cox in use runs is fine: the window runs the new
version once it is started again, and coxd too, once no chat is at work
as the window comes (see [coxd](#coxd)).

## Pull requests

Pull requests are merged with a merge commit, so every commit lands on
`main` as written and must pass `cargo xtask ci` on its own.

## Commit messages

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
A change confined to one crate uses the crate name as the scope, dropping a
leading `cox-`, as in `feat(core): ...` for `cox-core`. Other changes have no
scope.
