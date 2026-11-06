# Contributing

## Commands

- `cargo xtask setup` installs the pinned development tools.
- `cargo xtask ci` runs every check. A change is ready only when it passes.
- `cargo xtask install` builds Cox for release and installs it apart from
  `target/` (see [Using Cox to build Cox](#using-cox-to-build-cox)).
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

`cox [--stage <url>] [--home <dir>] [--data <dir>] <agent command>` opens a window with
no chat in it. The rail on the left of the window shows the projects,
each as a folder with its name under it, and its folder's path where
the mouse rests, and the tabs along the top show the chats of the project chosen
on the rail. A click on a project chooses it, showing the chat of it
shown last. A project is a folder: "Open folder…", the + at the foot of
the rail (Command-O on macOS, Control-O elsewhere), opens a chat in the
project of a folder of your choice, and the + after the tabs opens
another chat in the project chosen. "New chat" (Command-N, Control-N)
opens one in no project, which works in Cox's home: `<dir>` when given,
or else the folder the settings set, or else `~/Cox`; the rail shows it
as `~`. The gear at the foot of the rail (Command-, on macOS, Control-,
elsewhere) opens the settings, on a card over the window, where the
human sets the home: a path typed in its input, whole or from `~`, which
Enter sets, or a folder "Choose…" asks for. The settings are kept in
`settings.json`, in Cox's data folder (see [The stage](#the-stage)), as
`agents.json` is, and the
chats in no project that open from then on work there; `--home` still
goes ahead of them for its run, until the human sets the home. Escape,
the ×, or a click beside the card closes it. Each chat starts its own
`<agent command>` working in its project's folder, and has a stage of its
own, showing `<url>` first. Ahead of the first line sent in the chat, the
agent gets how the human works with Cox, whatever the project: the
`COX.md` of their workflow's folder, which `settings.json` names, whole
or from `~`, as `{"workflow": "~/dotfiles/cox"}`, or else `workflow`
beside it, or, when there is none, Cox's own
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
is discarded, with `{left_out}`, the files too big to keep. The tabs switch between chats (Control-Tab and
Control-Shift-Tab go along every project's). A chat is named by a number at first, then
after the first line sent in it, then by the title its agent gives its
session, if it gives one (ACP's `session_info_update`, as
`claude-agent-acp` sends), until the human renames it: a
double click on its tab, or "Rename" in the menu of the right button.
A dot on the tab marks how the chat stands: a ring while its agent is
at work, in the accent, or idle, in grey; filled when it waits for the
human, in amber, or finished a turn the human has not seen yet, in the
accent. A closed chat has none. The project's square on the rail counts
its chats that wait for the human or that they have not seen.
On macOS, a chat the human is not looking at, as when another chat is
shown or Cox is behind other windows, tells them with a notification
when it comes to wait for them, and when its turn ends; a click on the
notification shows the chat. The Dock's icon counts the chats that
wait for the human or that they have not seen since a turn ended.
"Archive" in the menu takes a chat out of the tabs, to "Archived" at
their end, from where a click brings it back; its agent, if it runs, goes on. A chat's agent runs until Cox quits. Cox
keeps each chat's conversation, and its stage's tabs, as they change,
and shows the chats of earlier runs as tabs too, greyed and closed, for the
human to read what they showed. A closed chat's stage opens again on
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

A pill at the right of the tabs, for the chat shown, shows what
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
`<agent command>`, which a new chat talks to, then those the human lists
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
other text is a goal in words; "Clear goal" clears it; the tab marks a
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
the tabs, and the human does not talk to it. Each time a turn ends, Cox
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
project's folder (or, as an earlier Cox read it, a `COX.goal.md`) replaces what the judge is told it is, for a project
that wants its goals judged in a way of its own; Cox still adds how to
end the reply, and the goal. When the judge finds that one command
decides the goal, passing just when it is met, it names the command on
a line `COMMAND: ` ahead of its verdict, and the chat proposes the
command as the goal, on a card as the agent's proposals are, once: set,
the command checks the goal from then on, the same way each time, and
the judge goes.

A chat's agent can open a chat too, with Cox's `open_chat` tool, giving
it its first line: the chat opens in a tab after the agent's own,
marked ↳, in the same project, and works on that line, which shows in its
conversation in the human's place but dimmer, after a line that says
which chat opened it. The agent says how the chat's agent is set up:
`context`, text the agent gets after the project's `COX.md` and ahead of
the first line, which the chat notes at its top; `tools`, the names of
the Cox tools it gets (`show_markdown`, `show_file`, `show_changes`, `show_map`, `run_command`,
`open_chat`, `wait_chat`, `propose_goal`, `start_sketch`, `wiki_search`, `wiki_read`, `wiki_write`), all of them unless given; `stage`, `drive` for the tools
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
of theirs, or else `wiki` in Cox's data folder. A chat's agent
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
not how the code works, and no secrets. What holds only of one project,
as of its own people, customers or plans, the agent keeps in it, with
`wiki_write`'s `keep_in_project`: the page's properties, the front
matter Obsidian keeps between `---` lines at its top, then say
`only: <project>`, by the project's name, and only that project's chats
find the page, read it, see it in the index, or write it. The human
can write the line themselves too.

So that the human need not remember to, Cox asks at a chat's
milestones, when its goal is met and when a sketch ends, on a card in
the conversation of the chat they go on in, whether to keep what was
learned in the wiki: "Save to wiki" asks the agent as the menu's does,
for a sketch the sketch's own agent, which found it, and goes on in the
project, and "Not now" lets it be. A goal is asked of once for a line
of the human's, so that the turn that keeps it, which meets the goal
again, does not ask again; and a chat whose agent has no `wiki_write`
is not asked.

"Wiki" on the stage's bar, a plugin (see below), at `cox://wiki/`,
shows the wiki to the human: its pages, with their titles, their first
lines and where they were used last, which a line at the top narrows to
those that have every word typed; and a page, at `cox://wiki/#` and its
name, as `cox://wiki/#rust/errors`, drawn as Markdown, its links going
to the pages they name, `[[name]]` or `[[name|what it says]]`, as
Obsidian writes them, or a Markdown link to a page's file, a name that
is no page's naming the only page whose last part it is, as Obsidian
finds it; then where it was used, the pages that link to it, and its
file, which a click opens on a tab. HTML a page holds shows as text,
so that what an agent wrote runs nothing on the stage. Cox marks, with
no agent, the pages with links to no page, those no page links to, the
index aside, those with no title, and those that say nowhere where they
were used, as those the human wrote; chips over the pages narrow them
to those of a mark. A page kept in one project says which, as the
human sees every page. The page reads the wiki again every two seconds.

The agents write the wiki in English, whatever language their chats
speak, as Cox's tools and `wiki/save.md` tell them, so that a page is
found by the same words from every chat; the human reads it, and
whatever else they select in the window, in a language of their own,
as `settings.json` names it, as `{"translation": {"language":
"Japanese"}}`. "Wiki" then shows a page in that language, as far as it
is translated, the rest as it is written, with how far it has gone,
and a chip "As written" over it to read it as the agent wrote it. It
is translated a block at a time, its paragraphs, headings, lists,
quotes and tables, as it is opened, and not its code or the HTML it
holds. Text selected anywhere in the window, in the conversation, an
input, a tab of code or Markdown, a terminal, or a page of the stage,
is translated with "Translate" in the menu the right button opens on
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
text.

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

A chat's stage holds pages in tabs, and opens on the left of the
conversation only when it is used: the round button at the right end
of the tabs, after the pill, marked with a window split in two, opens
and closes it (Command-\\, Control-\\ off macOS), and it opens on
its own when a link opens on it, or when the agent works on a page
there (navigates it, clicks or types in it, or
opens a tab), showing that page's tab. Closed, it keeps its tabs. The
bar along its top keeps the views of the project in sight first, side
by side: "Files", "Changes" and the plugins' pages, such as "Map" (see
below), each a tab that the bar shows by its button, not among the
others; then the other tabs, each with an icon of what it shows: a
page, a Markdown file, code, a terminal, the flow of a tool call. The +
at the bar's end opens a menu of what the human opens there: a line at
its top, where Enter opens what they typed, a file of that path, from
the chat's folder, if there is one or the line starts as a path does,
or else a page, when the line has no space and what comes before its
first `/` has a scheme, or a dot or a port, as a host has, or else that
file again, when the line has no space and a `/`, as `src/new.rs`, or
else a command, which runs in a terminal; then "Web page", a
new tab, "Terminal" (see below), and "File…", which asks for a file
and opens it as a link to it does. Escape closes the menu. Their keys, Command-T, Control-` and
Command-P (Control-T and Control-P off macOS), open them from anywhere
in the window, on the stage of the chat shown, which opens. The cursor over a page is the one
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
last commit. A click on a file, or on a line, opens the file there, as
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

"Map" opens a tab of the project's map, for the human to see
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
The map is a file in the project's folder, `.cox/map.json`, written by
an agent once and kept up to date by the agents that change the code,
so that it is there whenever the tab is opened, the same whatever
changed; a project tracks it or not as it likes, and Cox's own
repository does not, so a clone of Cox gets its map written where it
is. Over it Cox lays what it knows: what changed, as
"Changes" shows it, falls on the boxes that hold the files, and up on
the boxes that open their tiers, with how many lines changed in how
many files, so that the human sees where a change falls in the project
before they read it; and a box whose paths
the project no longer has is marked stale, for an agent to bring the
map up to date. A project with no map gets a tab that says so, and an
agent asked to show the map is told how the file is written. The agent opens
the map with Cox's `show_map` tool, at a tier, or a box in it, to show
the human where what it talks of is before it talks it through, and
hears what the map shows there, box by box. The tab follows the file
and the project as they change, and a closed chat's stage opens it
again where it was open. The map is a page of the stage, one of its
plugins (see below), at `cox://map/`, where the map is open being the
page's hash, as `cox://map/#cox/stage`.

A tab can show a plugin: a page, in HTML and JavaScript, that draws
what Cox knows as a tab Cox draws would, for whoever wants a view of
their own on the stage, the agent included, since it writes HTML and
reads pages. "Map" and "Wiki" are built in. A plugin is a
folder with an `index.html`, and a `plugin.json` naming its `title` if
it has one: those built into Cox, in `crates/cox/plugins`, and the
project's, in `.cox/plugins/<name>` under its folder. The bar has a
button for each, after "Changes", which opens the plugin's page at
`cox://<name>/`, an address the stage serves itself, through CEF, from
the plugin's files, or shows the tab that has it, wherever the page has
gone; the agent opens it as any page. A page at a `cox:` address is
drawn as a tab Cox draws, without the toolbar a page of the web has,
since the page is the tab. A plugin reads what Cox knows as JSON from
`cox://api/<what>`: `cox://api/map` is the map of the project, as its `.cox/map.json` has
it, with what Cox knows of each box laid over: the files it holds, as
the project has them now, what changed in them, and which are gone; and
`cox://api/file/<path>` is what the project's language server says of
the file at `<path>`, an absolute path in the project, once it has
answered, which the page reads again for: the symbols it defines, how
many places use each, and which files use them;
`cox://api/call/<path>:<line>` is what the server says a line of the
file calls, for a step of a flow made there;
`cox://api/wiki` is the human's wiki, its pages, each with what it
links to, what links to it, where it was used and what Cox marks of it,
and `cox://api/wiki/<page>` a page, with its text as HTML; and
`cox://api/lines/<path>:<line>` is the lines of the file around that
line, each as its stretches with what the syntax says each is, and
before them the lines that open what the line is within, as its
function's and its impl's, for a page to show the code a step is made
at, colored as the code tabs color it. It
opens a file on a tab
by going to `cox://open/<path>#L<line>`, which the stage takes as it
takes a link to a file (a page that is not itself a file may not link
to one). The Map plugin draws its own cards and lines, with no
library, and reads its JSON again every two seconds, as Cox's own tabs
follow what they show.

A tab can be a terminal too, through
[alacritty_terminal](https://crates.io/crates/alacritty_terminal).
"Terminal" in the menu of the stage's + opens one with the human's
shell, working in the chat's folder. The agent runs a command in one with Cox's `run_command`
tool: the stage shows it running, the human can type into it, as to
answer what it asks, and the agent gets what the terminal shows and the
exit code once it ends. A command gets the tab of the agent's last one
that ended, or a new tab. When the agent stops waiting for it, as when
the human stops the turn, the command gets Control-C. Copying and
pasting in a terminal is Command-C and Command-V on macOS, and
Control-Shift-C and Control-Shift-V elsewhere, where Control-N,
Control-O, Control-T, Control-P and Control-\\ go to the terminal too.

Cox hands each chat's agent its own tools as an MCP server, which is
`cox` itself run as `cox --mcp <socket>`: the agent starts it, and it
carries what the agent says to the chat on a Unix socket of the chat's
own, which Cox answers. An agent reaches only its own chat that way.
The sockets are in a folder of the run's own among the system's
temporary files, `cox-` and Cox's process id, which only the human may
enter, beside what else Cox keeps only while it runs, as a throwaway
profile of the stage; a Cox that crashes leaves its folder, which the
next Cox to start takes away.

CEF opens the Chrome DevTools Protocol on a port that is free when Cox
starts, so two Cox can run side by side, and Cox hands each chat's
agent the
[Chrome DevTools MCP server](https://www.npmjs.com/package/chrome-devtools-mcp)
connected to it, so the agent reads and drives the pages the human sees
in that chat. The server runs through `npx`, so Node has to be on
`PATH`; without it, the agent gets no tools for the stage. The server
connects through a relay in Cox, one for each chat, which passes
everything on but keeps the agent to its chat's pages: the agent hears
nothing of other chats' pages, and the pages it opens and closes are
tabs of its chat's stage. The relay also tells the stage where the agent
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
  `agents.json`, and the `workflow` and `wiki` folders when the settings
  name none; beside them `sketches`, the folders the sketches work in,
  and `logs`;
- in `state`, what Cox would lose: the chats, in a SQLite database,
  `chats.sqlite`, and the stage's profile, `stage`;
- in `cache`, what Cox can make again: `registry.json`, the agents it
  downloads, in `agents`, and `translations`;
- in `run`, what is there only while Cox runs: `inspect.sock`.

A Cox from before kept them all at the top of the folder, and a Cox
built in `target/` kept each beside it, as `cox-` and its name. Cox
moves them where they go as it starts, and the chats that work in a
sketch that moved follow it; while another Cox from the same place
runs, it leaves them, and uses them where they are.

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
starting from the executable's. It then signs the app as a whole, ad
hoc, which macOS asks of an app before it shows the app's
notifications. `cargo run` on macOS stops at once,
saying where the framework was expected.

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
it was said at. The files are in `logs`
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
[The stage](#the-stage)), and says where the files of its runs are. Its
chats in no project work in `home` there, as its settings set it the
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
files whole, and reads the files a line points to as
`claude-agent-acp` does, a link to each and its text, which a turn's
`when` can match: the tour answers a line that points to a file.
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
  in the order of the tabs, project by project, by the numbers they go
  by to the agents' tools, and one chat in full, the chat shown or chat `<chat>`: what it
  waits for, as its input shows while empty, its input, its stage's tabs, and its conversation, entry by
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

## Using Cox to build Cox

The Cox in use runs from where `cargo xtask install` puts it, so that
building and trying a change in `target/` leaves it be:

- On macOS, `~/Applications/Cox.app`: run
  `~/Applications/Cox.app/Contents/MacOS/Cox <agent command>`.
- On Linux, `~/.local/bin/cox`, with what it needs of CEF in
  `~/.local/lib/cox`.

A Cox started from it, such as `target/debug/Cox.app`, runs beside it
with a DevTools port and a stage profile of its own (see
[The stage](#the-stage)), so each agent drives its own stage. Installing
again while the Cox in use runs is fine: it runs the new version once it
is started again.

## Pull requests

Pull requests are merged with a merge commit, so every commit lands on
`main` as written and must pass `cargo xtask ci` on its own.

## Commit messages

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
A change confined to one crate uses the crate name as the scope, dropping a
leading `cox-`, as in `feat(core): ...` for `cox-core`. Other changes have no
scope.
