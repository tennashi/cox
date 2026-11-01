# Contributing

## Commands

- `cargo xtask setup` installs the pinned development tools.
- `cargo xtask ci` runs every check. A change is ready only when it passes.
- `cargo xtask install` builds Cox for release and installs it apart from
  `target/` (see [Using Cox to build Cox](#using-cox-to-build-cox)).

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

`cox [--stage <url>] [--home <dir>] <agent command>` opens a window with
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
conversation so far, as Cox kept it, ahead of that line.
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
the Cox tools it gets (`show_markdown`, `show_file`, `run_command`,
`open_chat`, `wait_chat`, `propose_goal`), all of them unless given; `stage`, `drive` for the tools
that read and drive the stage, the default, or `look` for none of them;
`allow` and `reject`, patterns of the titles of the permission requests
Cox answers in the human's place, as `Edit *` or `cargo test*`, where
`*` stands for any run of characters, a title both match being rejected,
and the rest asked of the human as usual, with the conversation saying
which Cox answered; and `goal`, what the chat works until it is met,
as "Set goal…" takes it: `$ ` and a command, or words. The titles are
the agent's, as the conversation shows them: `claude-agent-acp` titles a request after the
tool and what it works on, as `Edit src/main.rs` or `cargo xtask ci`. A
chat the human opens is set up with none of this: its agent gets every
tool, and the human answers every request. Which chat opened a chat, and
how its agent is set up, last across runs with the rest of the chat.
`open_chat` answers with a number the chat goes by for this run, which
the agent gives Cox's `wait_chat` tool to wait for the chat's turn to
end, or for the chat to come to wait for the human, and hear how it
stands then: what its agent last said, or what it asks the human. A
chat that is not at work answers at once.

The conversation shows each tool call the agent makes on a line of its
own, marked as running, done or failed; a click on the line opens what
the call showed, which shows from the start for a call that failed.
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
conversation gets a tab of its own, unless a tab shows it already.

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
one, started when a tab first needs it and shared by the tabs of all its
chats, until Cox quits; it builds what it needs in `target/rust-analyzer`,
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
in `target/debug/cox-chats.sqlite`.

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
