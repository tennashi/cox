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
no session in it. "New session" (Command-N on macOS, Control-N
elsewhere) opens one working in `<dir>`, `~/Cox` unless given, and "Open
folder…" (Command-O, Control-O) opens one in a folder of your choice.
Each session starts its own `<agent command>` working there, and has a
stage of its own, showing `<url>` first. The list on the left of the
window switches between sessions (Control-Tab and Control-Shift-Tab do
too) and closes them. Escape in a session's conversation, or its Stop
button, stops the turn its agent is taking; the conversation stays, and
the next line can be sent once the agent has stopped. A click on a link
in a session's conversation opens it on that session's stage.

CEF opens the Chrome DevTools Protocol on a port that is free when Cox
starts, so two Cox can run side by side, and Cox hands each session's
agent the
[Chrome DevTools MCP server](https://www.npmjs.com/package/chrome-devtools-mcp)
connected to it, so the agent reads and drives the page the human sees
in that session. The server runs through `npx`, so Node has to be on
`PATH`; without it, the agent gets no tools for the stage. The server
connects through a relay in Cox, one for each session, which passes
everything on but keeps the agent to its session's page: the agent hears
nothing of other sessions' pages and cannot open pages of its own. The
relay also tells the stage where the agent points, clicks and types, so
the stage can draw the agent's pointer over the page.

The stage keeps its cookies, logins and storage in a profile that lasts
across runs, one for each place Cox runs from. The Cox that
`cargo xtask install` puts in place keeps it in
`~/Library/Application Support/Cox/stage` on macOS and
`~/.local/share/cox/stage` on Linux; any other, such as
`target/debug/Cox.app`, keeps its own in `cox-stage` beside it, as in
`target/debug/cox-stage`. Only one Cox at a time can use a profile; a
second Cox from the same place gets a profile for that run only.

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
starting from the executable's. `cargo run` on macOS stops at once,
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
