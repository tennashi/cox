# Contributing

## Commands

| Command | What it does |
| --- | --- |
| `cargo xtask setup` | Installs the pinned development tools. |
| `cargo xtask ci` | Runs every check: `cargo fmt`, `clippy`, the tests and `cargo deny`. A change is ready only when it passes. |
| `cargo xtask run` | Builds Cox and runs it with a fake agent, apart from your own data. See [Trying a change](#trying-a-change). |
| `cargo xtask bundle [--release]` | Builds `Cox.app` on macOS. See [Building on macOS](#building-on-macos). |
| `cargo xtask install` | Builds Cox for release and installs it. See [Using Cox to build Cox](#using-cox-to-build-cox). |

## Code layout

| Crate | What it holds |
| --- | --- |
| [`cox`](crates/cox) | The app: the window (GPUI), `coxd`, the chats, the stage, and Cox's tools for agents. |
| [`cox-core`](crates/cox-core) | What Cox keeps of your work (projects, chats, what each showed), without GPUI or CEF. |
| [`cox-acp`](crates/cox-acp) | Talking to an agent over ACP. |
| [`cox-browser`](crates/cox-browser) | The stage's browsers: CEF rendering pages with no window of their own. |
| [`cox-editor`](crates/cox-editor) | Files as the code tabs hold them: text, syntax (tree-sitter) and language servers. |
| [`cox-android`](crates/cox-android) | The Android emulator on the stage, over its gRPC service. |
| [`cox-mac`](crates/cox-mac) | Other Mac apps' windows on the stage, through ScreenCaptureKit. |
| [`cox-fake-agent`](crates/cox-fake-agent) | An ACP agent that plays a scenario, for tests and for trying Cox. |
| [`xtask`](xtask) | The `cargo xtask` commands. |

Each module starts with a doc comment (`//!`) that says what it does and
why; read it before changing the module, and keep it true when you do.

## Building

The stage is rendered by CEF (Chromium) through the
[cef](https://crates.io/crates/cef) crate, whose build downloads the
matching CEF distribution (about 200 MB, once per target directory).

### Building on Linux

gpui links these system libraries (Debian and Ubuntu package names):

```sh
sudo apt-get install libvulkan-dev libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev
```

`cargo run` is enough: CEF is linked into `cox`, and its helper processes
run `cox` again.

`Cargo.lock` holds `libc` at 0.2.188: gpui depends on `xattr` 0.2, which
does not build with later `libc` releases on Linux. Do not update `libc`
until gpui moves off that `xattr`.

### Building on macOS

The Command Line Tools are enough; Xcode is not needed. `cox` turns on
gpui's `runtime_shaders` feature, which compiles the Metal shaders when
the app starts rather than at build time, so Xcode's `metal` compiler is
not needed. The cef crate's build compiles a C++ wrapper with CMake,
which needs `cmake` and `ninja` (`brew install cmake ninja`).

On macOS, CEF is a framework the app loads when it starts, and its helper
processes are small apps of their own, so Cox has to run from an app
bundle. `cargo run` stops at once, saying where the framework was
expected. Build the bundle instead:

```sh
cargo xtask bundle            # or: cargo xtask bundle --release
target/debug/Cox.app/Contents/MacOS/Cox <agent command>
```

`cargo xtask bundle` builds `cox` and `cox-helper`, then lays them out as
`Cox.app` next to them: `cox` as `Contents/MacOS/Cox`, and the framework
and five copies of `cox-helper` (`Cox Helper.app`,
`Cox Helper (GPU).app`, ...) under `Contents/Frameworks`. CEF fixes that
layout and those names. It renders the app's icon,
[`app-icon.svg`](crates/cox/assets/app-icon.svg), into
`Contents/Resources/Cox.icns`.

Last, it signs the app, which macOS requires before it shows the app's
notifications. It signs with the certificate `COX_SIGNING_IDENTITY`
names, or else with "Cox Local Signing" if the keychain has it, or else
ad hoc. macOS remembers what you allowed an app (such as recording the
screen) by its signature, and an ad hoc signature changes with every
build, so macOS asks again after each one. To avoid that, make a
certificate once in the login keychain with Keychain Access →
Certificate Assistant → Create a Certificate…, named "Cox Local
Signing", with the identity type "Self Signed Root" and the certificate
type "Code Signing". The first build to sign with it asks for the login
keychain's password once for each part of the app it signs; "Always
Allow" lets it from then on.

## Trying a change

```sh
cargo xtask run [--headless] [--data <dir>] [<scenario> | -- <agent command>...]
```

builds Cox and runs it with the fake agent playing `<scenario>` (the
[tour](crates/cox-fake-agent/scenarios/tour.json) unless given), or with
`<agent command>`. It keeps Cox's data in `<dir>`, or afresh in
`target/debug/cox-run` each time, and prints where the run's files are.
Cox's own chats work in `home` there.

Builds other than the installed one keep their data in `cox-data` beside
the binary, such as `target/debug/cox-data`, so they never touch the data
of the Cox you use. See [the data folder](docs/configuration.md#the-data-folder).

### Without a screen

On Linux, `--headless` runs Cox on a display of its own, which Xvfb draws
in memory. `cox --inspect` (see [Inspecting a running Cox](#inspecting-a-running-cox))
looks at it and types into it:

```sh
sudo apt-get install xvfb mesa-vulkan-drivers xdotool imagemagick
cargo xtask run --headless
target/debug/cox --data target/debug/cox-run --inspect key ctrl-n
target/debug/cox --data target/debug/cox-run --inspect type 'hello'
target/debug/cox --data target/debug/cox-run --inspect key enter
target/debug/cox --data target/debug/cox-run --inspect state
target/debug/cox --data target/debug/cox-run --inspect screenshot shot.png
```

gpui draws with Vulkan, which `mesa-vulkan-drivers` provides on a machine
with no GPU.

## The fake agent

`cox-fake-agent <scenario>` is an agent for when no model should answer:
to try Cox out, to see a change working, or to reproduce a fault. It
speaks ACP as `claude-agent-acp` does, and answers each line sent to it
with a turn of the scenario, a JSON file.

A scenario has:

| Key | Meaning |
| --- | --- |
| `turns` | The turns the agent plays (see below). |
| `configOptions` | The settings of each session, as ACP's `configOptions`. The agent sets them as Cox asks. The tour has a mode, a model and an effort. |
| `commands` | The slash commands the agent has, as ACP's `availableCommands`. The tour has three, and answers `/compact`. |
| `authMethods` | The ways to log in, as ACP's `initialize` gives them. With them, the agent asks Cox to log in before it opens a session, or, with `"authRequired": "prompt"`, when it takes a turn. |

Each line takes the first turn not taken yet whose `when`, if any,
appears in the line. A turn takes its steps in order:

| Step | What it does |
| --- | --- |
| `say` | Sends text. |
| `think` | Sends a thought. |
| `tool`, `tool_update` | ACP's `tool_call` and `tool_call_update`, with `id` for `toolCallId`. |
| `update` | Any ACP `session/update`, as it is. |
| `ask` | ACP's `session/request_permission`, with `then`: the steps for each option, by its `optionId`, or `cancelled`. |
| `call` | Calls a tool of an MCP server Cox hands the agent (`cox` unless `server` says otherwise), with `arguments`, titled `title` (the tool's name by default). |
| `sleep` | Waits, in milliseconds. |
| `stderr` | Writes to standard error. |
| `fail` | Fails the turn. |

In a step, `{cwd}` stands for the session's folder and `{result}` for
what the last call answered; a value that is just
`"{result:number after as chat }"` takes the number after `as chat ` in
it, as `open_chat` answers, to pass to `wait_chat`. The turn ends with
its `stop` (`end_turn` by default), or `cancelled` when Cox cancels it.

The agent accepts embedded files and pictures, and reads the files a
line points to as `claude-agent-acp` does, so a turn's `when` can match
them. It takes a line Cox steers into a running turn as
`claude-agent-acp` does, playing that line's turn after the current
turn's own steps. A way to log in of `"type": "terminal"` with
`"args": ["--log-in"]` runs `cox-fake-agent <scenario> --log-in`, which
logs the agent in for later runs, until its file in
`cox-fake-agent-logins` in the system's temporary folder is removed.

`cox-fake-agent --from-log <run> [--chat <id>]` writes, as a scenario,
what a chat's agent did in a [logged run](#logs) (`judge` for a goal's
judge): its updates, permission requests and standard error, turn by
turn, with the waits between them, so the run can be replayed, such as
on another machine. Calls of Cox's own tools do not go over ACP, so the
scenario does not make them.

## The fake Android emulator

`cox-fake-emulator` stands in for the Android emulator where none runs,
such as on a machine without virtualization. It takes the arguments Cox
starts `emulator` with, writes the file an emulator writes about itself,
and serves the part of the emulator's gRPC service Cox uses, drawing a
phone screen of its own. `--make-avd` writes a device for it, "Fake
Phone", where `ANDROID_AVD_HOME` points:

```sh
cargo build --bin cox-fake-emulator
mkdir -p target/debug/fake-android-sdk/emulator
ln -sf ../../cox-fake-emulator target/debug/fake-android-sdk/emulator/emulator
export ANDROID_HOME=$PWD/target/debug/fake-android-sdk
export ANDROID_AVD_HOME=$PWD/target/debug/fake-android-avd
target/debug/cox-fake-emulator --make-avd
cargo xtask run --headless
```

## Inspecting a running Cox

Cox answers questions about its window to an agent that debugs it, in a
terminal, in a chat of another Cox, or on a machine with no screen. When
the window opens, it listens on a Unix socket, `run/inspect.sock` in its
data folder. Keys, text, clicks and actions sent there reach the window
whether it has focus or not, such as when the Cox under test runs behind
the Cox you use. The socket speaks MCP, so `cox --mcp <socket>` reaches it
as an MCP server, for an agent set up with one, as in Claude Code's
`.mcp.json`.

```sh
cox [--data <dir>] --inspect [<tool> [<arguments>...]]
```

asks the Cox whose data folder is this `cox`'s, or `<dir>`, and prints
the answer. With no tool, it lists the tools:

| Tool | What it does |
| --- | --- |
| `state [<chat>]` | Where Cox keeps its data and this run's log, the stages' DevTools port, the window and keyboard focus, the chats by project with the numbers agents' tools know them by, and one chat in full (the one shown, or `<chat>`): what it waits for, its input, its stage's tabs and its conversation. |
| `screenshot [<file>]` | Writes the window as a PNG (`cox-screenshot.png` by default), through ImageMagick's `import` on Linux and `screencapture` on macOS, which must be allowed to record the screen. |
| `key <keystroke>...` | Presses keys as GPUI names them, such as `ctrl-n`, `enter`, `escape`, `1`, or `secondary-o` (Cmd on macOS, Ctrl elsewhere), and says whether each was handled. |
| `type <text>` | Types text one character at a time; a newline presses Enter. |
| `click <x> <y> [left\|right\|middle] [<count>]` | Clicks a point of the window, in points from its top left. Through `xdotool` on Linux; not on macOS yet. |
| `action <name>` | Dispatches a GPUI action by name, such as `cox::NewChat`, or `NewChat` when the name is unambiguous. |
| `frames [<seconds>]` | Where the window stuck over the last ten seconds (up to a minute): each time the window's thread was busy for 100 ms or more, and, for each page of the stage, how often the wheel scrolled it, how many frames CEF painted, and each wait of 50 ms or more from a wheel turn to the next frame. A page scrolled to its end paints nothing either. |

## Logs

Cox writes errors to standard error, or what `RUST_LOG` asks for. It also
keeps everything down to `debug` (or what `COX_LOG` asks for) in a file
for each run, so an agent can read and search it after the run: one JSON
object per line, with the spans it was logged in (such as the chat) and
the file and line of Cox's code that logged it. The files are in `logs`
in the data folder, such as `target/debug/cox-data/logs`, named after the
run's start time and process id, as `20261101T160000Z-4242.jsonl`. Cox
keeps the last twenty runs. A panic is logged with its backtrace.

Useful targets:

| Target | What it logs |
| --- | --- |
| `cox_acp::wire` | Every ACP message between Cox and an agent. |
| `cox_acp::stderr` | What an agent writes to standard error. |
| `cox::agent::mcp::wire` | Every message between an agent and Cox's tool server. |
| `cox::agent::mcp::asks` | What a tool asks of the stage. |
| `cox::stage::asked` | The stage carrying it out. |
| `cox_browser::scheme` | How long each answer at a `cox:` address took; a warning at 100 ms or more. |
| `cox::busy` | Each time the window's thread (which also gives CEF its turns on macOS) was busy for 100 ms or more. |

## Using Cox to build Cox

`cargo xtask install` puts the Cox you use apart from `target/`, so
building and trying a change leaves it alone:

- On macOS, `~/Applications/Cox.app`.
- On Linux, `~/.local/bin/cox`, with what it needs of CEF in
  `~/.local/lib/cox`.

It also has that Cox's `coxd` start when you log in: on macOS through a
launch agent, `~/Library/LaunchAgents/com.github.tennashi.cox.coxd.plist`,
which also starts it at once; on Linux through a systemd user unit,
`~/.config/systemd/user/coxd.service`, from the next login. To stop
that, run `launchctl bootout gui/$(id -u)/com.github.tennashi.cox.coxd`
and remove the file, or run `systemctl --user disable coxd.service`.

A Cox built in `target/` runs beside the installed one, with its own data
folder, DevTools port and browser profile, so each agent drives its own
stage. Installing again while the installed Cox runs is fine: the window
runs the new version once it is started again, and `coxd` once no chat is
at work when a window connects.

## Pull requests

Pull requests are merged with a merge commit, so every commit lands on
`main` as written and must pass `cargo xtask ci` on its own.

When a change alters a file format users write, update
[docs/configuration.md](docs/configuration.md) in the same pull request.

## Commit messages

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
A change confined to one crate uses the crate name as the scope, dropping
a leading `cox-`, as in `feat(core): ...` for `cox-core`. Other changes
have no scope.
