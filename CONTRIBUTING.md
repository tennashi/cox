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
no chat in it. The list on the left of the window shows the chats under
the projects they are in. A project is a folder: "Open folder…"
(Command-O on macOS, Control-O elsewhere) opens a chat in the project of a
folder of your choice, and the + beside a project opens another chat in
it. "New chat" (Command-N, Control-N) opens one in no project, which
works in `<dir>`, `~/Cox` unless given. Each chat starts its own
`<agent command>` working in its project's folder, and has a stage of its
own, showing `<url>` first. When the folder has a `COX.md`, the agent gets
what it says ahead of the first line sent in the chat, and the chat says
so at its top. The list switches between chats (Control-Tab and
Control-Shift-Tab do too). A chat is named by a number at first, then
after the first line sent in it, then by the title its agent gives its
session, as `claude-agent-acp` does, until the human renames it: a
double click on its row, or "Rename" in the menu of the right button.
The row marks a chat whose agent is at work (…), one that waits for the
human (?), and one that finished a turn the human has not seen yet (●).
On macOS, a chat the human is not looking at, as when another chat is
shown or Cox is behind other windows, tells them with a notification
when it comes to wait for them, and when its turn ends; a click on the
notification shows the chat. The Dock's icon counts the chats that
wait for the human or that they have not seen since a turn ended.
"Archive" in the menu takes a chat out of the list, to "Archived" at its
foot, from where a click brings it back; its agent, if it runs, goes on. A chat's agent runs until Cox quits. Cox
keeps each chat's conversation, and its stage's tabs, as they change,
and lists the chats of earlier runs too, greyed and closed, for the
human to read what they showed. A closed chat's stage opens again on
the pages and Markdown files it had; its terminals do not start again.
A line sent in a closed chat starts its agent again, which goes on with
the chat's session when it can (ACP's `session/resume`, or else
`session/load`); otherwise the agent starts a new session, and gets the
conversation so far, as Cox kept it, ahead of that line, and the chat
notes that the agent could not resume. A chat with no session to go on
with, as one that moved into a trial or out of one, starts a new
session the same way, without the note.
Escape in a chat's
conversation, or its Stop button, stops the turn its agent is taking; the
conversation stays, and the next line can be sent once the agent has
stopped. A click on a link in a chat's conversation opens it on that
chat's stage.

A chat can have a goal, which the chat works until it is met: a
command, such as `cargo xtask ci`, which has to pass, or words, such as
"The greeting names the user.", which an agent judges. "Set goal…" in
the menu of the right button opens an input above the chat's own for
the goal (Enter sets it, Escape leaves it as it was, and emptied, it
clears the goal), where `$ ` and a command is the command, and any
other text is a goal in words; "Clear goal" clears it; the row marks a
chat with a goal (◎), and a line above the input names the goal while
the chat waits. When a turn ends as the agent ends it, a command runs
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
permission requests, Cox allows those that read, search, or run the
usual tests (`Read *`, `Grep *`, `git diff*`, `cargo test*`, and so
on), and cancels the rest, since no human answers for it. The judge
keeps one session while the goal lasts, so it remembers what it looked
at, until the goal changes or Cox quits. A `COX.goal.md` in the
project's folder replaces what the judge is told it is, for a project
that wants its goals judged in a way of its own; Cox still adds how to
end the reply, and the goal. When the judge finds that one command
decides the goal, passing just when it is met, it names the command on
a line `COMMAND: ` ahead of its verdict, and the chat proposes the
command as the goal, on a card as the agent's proposals are, once: set,
the command checks the goal from then on, the same way each time, and
the judge goes.

A chat's agent can open a chat too, with Cox's `open_chat` tool, giving
it its first line: the chat opens under the agent's own in the list, in
the same project, and works on that line, which shows in its
conversation in the human's place but dimmer, after a line that says
which chat opened it. The agent says how the chat's agent is set up:
`context`, text the agent gets after the project's `COX.md` and ahead of
the first line, which the chat notes at its top; `tools`, the names of
the Cox tools it gets (`show_markdown`, `show_file`, `show_changes`, `show_map`, `run_command`,
`open_chat`, `wait_chat`, `propose_goal`, `start_trial`), all of them unless given; `stage`, `drive` for the tools
that read and drive the stage, the default, or `look` for none of them;
`allow` and `reject`, patterns of the titles of the permission requests
Cox answers in the human's place, as `Edit *` or `cargo test*`, where
`*` stands for any run of characters, a title both match being rejected,
and the rest asked of the human as usual, with the conversation saying
which Cox answered; and `goal`, what the chat works until it is met,
as "Set goal…" takes it: `$ ` and a command, or words; and `trial`,
which makes the chat a trial, from the project (`project`) or from
nothing (`empty`). The titles are
the agent's, as the conversation shows them: `claude-agent-acp` titles a request after the
tool and what it works on, as `Edit src/main.rs` or `cargo xtask ci`. A
chat the human opens is set up with none of this: its agent gets every
tool, and the human answers every request. Which chat opened a chat, and
how its agent is set up, last across runs with the rest of the chat.
A trial tries the work out apart from the project, before the human and
the agent agree what it is to be. A chat's agent makes its own chat a
trial with Cox's `start_trial` tool, saying what the trial starts from,
`project` or `empty`, and what it tries: once its turn ends, its agent
starts again in the trial's folder, with the conversation so far, told
what it tries, and the row marks the chat as a trial. `trial` in
`open_chat` opens a chat that is a trial instead, for trying several
ways side by side, one trial each. A trial's agent works in a
folder Cox makes for it in `trials`, beside the chats' database (see
[The stage](#the-stage)), and changes and tests the files there
without asking the human, the policy allowing edits and what the judge
of a goal may run. A trial from the project works in a Git worktree of
the project's last commit, so the project has to be in a Git
repository; a trial from nothing works in an empty folder, `scratch-`
and a number, made a Git repository of its own, for a project of any
kind or none, which "Open folder…" opens as a project once the human
wants to go on with it. The chat notes where it works from then on,
and the agent that opened a trial with `open_chat` is told, to read
what the trial made and propose a goal from it. A chat a trial opens works in the trial's
folder too, and a trial goes on there once it is started again. The
menu of the right button on a trial ends it once the human has what
they wanted of it: "Keep as branch", for a trial from the project,
commits what it changed and keeps it as a branch of the project's
repository, `trial/` and the name of its folder, for the human to merge
or not, and takes the worktree away; "Open as project", for a trial
from nothing, opens its folder as a project, with a chat in it, to go on
with what it made; and "Discard trial" takes its folder away, unless
it was opened as a project, which the human goes on with. A trial
kept or discarded stops its agent, and those of the chats it opened,
which worked in its folder, and each chat goes on in the project: a
line sent starts its agent again there, with the conversation so far,
asking the human as any chat does, since what it was allowed was for
the trial.
`open_chat` answers with a number the chat goes by for this run, which
the agent gives Cox's `wait_chat` tool to wait for the chat's turn to
end, or for the chat to come to wait for the human, and hear how it
stands then: what its agent last said, or what it asks the human. A
chat that is not at work answers at once.

The conversation shows each tool call the agent makes on a line of its
own, marked as running, done or failed; a click on the line opens what
the call showed, which shows from the start for a call that failed.
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
server answers, and the agent says the call is done; for another tool,
such as the agent's own `Read`, the flow shows only what the agent says
of it, and its permission request and Cox's answer when it asks one.
What the agent thinks on its way, when it sends it, shows on a line of
its own too, open while it arrives, and so does the plan it works to,
with each step marked as waiting, under way or done. The plan also
stays in sight on a card over the top right of the conversation, after
its turn too, until the next line is sent; a click on the card's line
folds it to that line. The tool calls, thoughts and
plan of the agent one after another fold into one line, under which
only the calls still running and the thought still arriving show, until
a click opens it. When the agent asks permission, a click on an option,
or its number, answers. A line above the input tells what the chat waits
for, while it is not the next line to send.

`claude-agent-acp` sends the plan Claude Code keeps with its task tools,
which recent models get only when the agent runs with
`CLAUDE_CODE_ENABLE_TODO_TOOLS=1`. It sends what Claude thinks only with
`CLAUDE_CODE_THINKING_DISPLAY=summarized`, since recent models leave it
out unless asked. So, for both:

```sh
CLAUDE_CODE_ENABLE_TODO_TOOLS=1 CLAUDE_CODE_THINKING_DISPLAY=summarized \
  cox npx -y @agentclientprotocol/claude-agent-acp
```

A chat's stage holds pages in tabs, and opens beside the conversation
only when it is used: "Show stage" above the conversation opens it, and
it opens on its own when a link opens on it, or when the agent works on
a page there (navigates it, clicks or types in it, or opens a tab),
showing that page's tab. Closed, it keeps its tabs. The + beside the
tabs opens another, and a page that asks for a window of its own, as a
link with `target="_blank"` does, gets a tab instead. A link from the
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

A tab of Rust code asks `rust-analyzer` what the code means, which has to
be on `PATH` (`rustup component add rust-analyzer`). Each project gets
one, started when a tab, or the map, first needs it and shared by the
tabs of all its chats, until Cox quits; it builds what it needs in `target/rust-analyzer`,
so that it does not hold up the project's builds. The mouse resting on a
name shows what the server says of it, its type and documentation. A
click on a name with Command held, Control off macOS, goes to where it
is defined, as "Go to definition" in the menu of the right button does;
"Find references" there lists, under the code, where the name is used,
to go to each in turn. Going to a place in another file, the tab shows
that file: the ‹ and › beside its path go back and forward between the
places it went to, as Command-[ and Command-] do (Alt-Left and
Alt-Right, and Control off macOS), and the mouse's buttons for that.

"Files" beside the tabs opens a tab of the project's files as a tree,
without what Git ignores, for the human to open one: a click on a
folder opens or closes it, and a click on a file opens it, as a link to
it does. The tab follows the folders open in it as files come and go,
and a closed chat's stage opens it again with the same folders open.

"Changes" beside it opens a tab of what changed in the project's files,
as Git tells it, file by file and line by line, with the files Git does
not track yet but does not ignore: in a trial, all it changed since it
started, what it committed too, and elsewhere what changed since the
last commit. A click on a file, or on a line, opens the file there, as
a link to it does. The tab follows the files as they change. The agent
opens it with Cox's `show_changes` tool, to show the human what it
made, and hears which files changed; given another folder, such as that
of a trial it opened, the tab shows what changed there. A trial marks
where it starts with a ref of its worktree's own,
`refs/worktree/cox-start`: the commit it starts from, or the empty tree
for a trial from nothing, which then counts all it made; a trial opened
as a project forgets it.

"Map" beside it opens a tab of the project's map, for the human to see
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
the file defines, as `rust-analyzer` lists them for Rust, each opening
the file at its line, how many places use each, and which files use
them; until the server has answered, the file says what the server is
doing. The path above the boxes leads back up. A map can have flows
too, the orders in which the boxes talk, each a list of steps from one
box to another, written between the boxes that hold the code: "flows"
above the boxes lists them, and a flow chosen is drawn as a sequence
diagram of the tier that is open: the boxes its steps go between, rolled
up to the tier's, as columns in the order the tier draws them, each
with a lifeline, and the steps as rows from the top, an arrow from the
box that talks to the one it talks to, with what the step says; a step
inside a box is a loop on its lifeline, which opens the box at the
step, so a flow is read from the top down as the map is, and a column
opens its box as a card does. "on the map" draws the flow on the tier
instead, its steps numbered on the lines between the boxes, with the
step chosen lit and the other lines dimmed. Either way the ← and →
keys go from step to step.
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
reads pages. "Map" is the one built in. A plugin is a
folder with an `index.html`, and a `plugin.json` naming its `title` if
it has one: those built into Cox, in `crates/cox/plugins`, and the
project's, in `.cox/plugins/<name>` under its folder. The tab bar has a
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
many places use each, and which files use them. It
opens a file on a tab
by going to `cox://open/<path>#L<line>`, which the stage takes as it
takes a link to a file (a page that is not itself a file may not link
to one). The Map plugin draws its own cards and lines, with no
library, and reads its JSON again every two seconds, as Cox's own tabs
follow what they show.

A tab can be a terminal too, through
[alacritty_terminal](https://crates.io/crates/alacritty_terminal). The
`>_` beside the tabs opens one with the human's shell, working in the
chat's folder. The agent runs a command in one with Cox's `run_command`
tool: the stage shows it running, the human can type into it, as to
answer what it asks, and the agent gets what the terminal shows and the
exit code once it ends. A command gets the tab of the agent's last one
that ended, or a new tab. When the agent stops waiting for it, as when
the human stops the turn, the command gets Control-C. Copying and
pasting in a terminal is Command-C and Command-V on macOS, and
Control-Shift-C and Control-Shift-V elsewhere, where Control-N and
Control-O go to the terminal too.

Cox hands each chat's agent its own tools as an MCP server, which is
`cox` itself run as `cox --mcp <socket>`: the agent starts it, and it
carries what the agent says to the chat on a Unix socket of the chat's
own, which Cox answers. An agent reaches only its own chat that way.

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
the page.

The stage keeps its cookies, logins and storage in a profile that lasts
across runs, one for each place Cox runs from. The Cox that
`cargo xtask install` puts in place keeps it in
`~/Library/Application Support/Cox/stage` on macOS and
`~/.local/share/cox/stage` on Linux; any other, such as
`target/debug/Cox.app`, keeps its own in `cox-stage` beside it, as in
`target/debug/cox-stage`. Only one Cox at a time can use a profile; a
second Cox from the same place gets a profile for that run only. The
chats are kept the same way, in a SQLite database: `chats.sqlite` beside
the installed Cox's `stage`, and `cox-chats.sqlite` beside any other, as
in `target/debug/cox-chats.sqlite`. A Cox given `--data <dir>` keeps all
of that, and the rest it keeps, in `<dir>` instead, without the `cox-`:
`<dir>/stage`, `<dir>/chats.sqlite`, and so on, to run apart from what it
kept before, or on a copy of what another Cox kept.

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
beside the chats' database (see [The stage](#the-stage)), as
`~/Library/Application Support/Cox/logs` for the installed Cox on macOS,
and `cox-logs` beside any other, as `target/debug/cox-logs`, each named
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
[The stage](#the-stage)), and says where the files of its runs are.
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

Each line takes the first turn not taken yet whose `when`, if it has one,
is in what the line says, as a goal's judge is told, and the turn takes
its steps one after another: `say`, `think`, `tool` and `tool_update`
(the fields of ACP's `tool_call` and `tool_call_update`, with
`toolCallId` as `id`), `update` (any of ACP's `session/update`, as it
is), `ask` (ACP's `session/request_permission`, with `then`, the steps
for each option chosen, by its `optionId`, or `cancelled`), `call` (a
tool of an MCP server Cox hands the agent, `cox` unless `server` says
another, with `arguments`), `sleep` (milliseconds), `stderr`, and
`fail`. In a step, `{result}` stands for what the last call answered,
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
screen. It opens a Unix socket, `inspect.sock` beside the chats'
database (see [The stage](#the-stage)), as its window opens, and
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
  as the list shows them, by the numbers they go by to the agents' tools,
  and one chat in full, the chat shown or chat `<chat>`: the line above
  its input, its input, its stage's tabs, and its conversation, entry by
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
