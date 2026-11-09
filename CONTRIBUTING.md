# Contributing

## Setting up

You need:

- Rust, as [rustup](https://rustup.rs) installs it: it takes the version
  `rust-toolchain.toml` pins.
- On Linux, the system libraries (Debian and Ubuntu package names):

  ```sh
  sudo apt-get install libvulkan-dev libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev
  ```

- On macOS, the Command Line Tools, `cmake` and `ninja`:

  ```sh
  xcode-select --install
  brew install cmake ninja
  ```

- Node.js, only to run a real agent with `npx`.

Then:

1. Install the tools `cargo xtask ci` runs:

   ```sh
   cargo xtask setup
   ```

2. On a Mac, make a signing certificate, or macOS asks for its
   permissions (such as recording the screen) again after every build:

   ```sh
   cargo xtask identity new <folder>
   ```

   It puts the certificate in the login keychain and writes it into
   `<folder>`. To sign with the same certificate on another Mac, run
   `cargo xtask identity import <folder>` there. To sign with another
   certificate, name it in `COX_SIGNING_IDENTITY`.

3. Build Cox and run it with [the fake agent](#the-fake-agent). The first
   build downloads CEF (Chromium), about 200 MB.

   ```sh
   cargo xtask run
   ```

`cargo xtask help` lists the other development tasks.

## Investigating

### The fake agent

`cargo xtask run` runs Cox with `cox-fake-agent`, which plays a
scenario: a JSON file of what the agent does in each turn, such as the
[tour](crates/cox-fake-agent/scenarios/tour.json) it plays by default.
What a scenario can say is in the doc comment of
[`scenario.rs`](crates/cox-fake-agent/src/scenario.rs). To run a real
agent instead, give its command after `--`:

```sh
cargo xtask run -- npx -y @agentclientprotocol/claude-agent-acp
```

Cox keeps its data in `target/debug/cox-run`, apart from the Cox you use.

To make a scenario from a real run, run:

```sh
cox-fake-agent --from-log <run> [--chat <id>]
```

with a run's [log file](#logs) and the chat's number.

### Without a screen (Linux)

1. Install Xvfb, a software Vulkan driver, `xdotool` and ImageMagick:

   ```sh
   sudo apt-get install xvfb mesa-vulkan-drivers xdotool imagemagick
   ```

2. Run Cox on a display of its own:

   ```sh
   cargo xtask run --headless
   ```

3. Drive it and look at it with [`cox --inspect`](#inspecting-a-running-cox):

   ```sh
   target/debug/cox --data target/debug/cox-run --inspect key ctrl-n
   target/debug/cox --data target/debug/cox-run --inspect type 'hello'
   target/debug/cox --data target/debug/cox-run --inspect key enter
   target/debug/cox --data target/debug/cox-run --inspect state
   target/debug/cox --data target/debug/cox-run --inspect screenshot shot.png
   ```

### With the fake Android emulator

1. Build the fake emulator and lay it out as an Android SDK:

   ```sh
   cargo build --bin cox-fake-emulator
   mkdir -p target/debug/fake-android-sdk/emulator
   ln -sf ../../cox-fake-emulator target/debug/fake-android-sdk/emulator/emulator
   ```

2. Point Cox at it, and make a device, "Fake Phone":

   ```sh
   export ANDROID_HOME=$PWD/target/debug/fake-android-sdk
   export ANDROID_AVD_HOME=$PWD/target/debug/fake-android-avd
   target/debug/cox-fake-emulator --make-avd
   ```

3. Run Cox in the same shell:

   ```sh
   cargo xtask run --headless
   ```

### Inspecting a running Cox

```sh
cox [--data <dir>] --inspect [<tool> [<arguments>...]]
```

asks a running Cox about its window and drives it, whether the window has
focus or not. It reaches the Cox whose data folder is this `cox`'s, or
`<dir>`. With no tool, it lists the tools.

To give an agent these tools, set up `cox --mcp <data folder>/run/inspect.sock`
as an MCP server, as in Claude Code's `.mcp.json`.

### Logs

Each run's log is a file of JSON lines in `logs` in the data folder, such
as `target/debug/cox-run/logs`. `COX_LOG` sets what goes into it, and the
doc comment of [`logs.rs`](crates/cox/src/logs.rs) lists the targets
worth asking for.

## Making a pull request

1. Make a branch from `main`.
2. Make each commit pass `cargo xtask ci` on its own.
3. Open the pull request. It is merged with a merge commit.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
For a change confined to one crate, use the crate name without a leading
`cox-` as the scope, as in `feat(core): ...` for `cox-core`. Give other
changes no scope.
