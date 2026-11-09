# Cox

Cox is a desktop app for working with coding agents. You talk to agents
such as Claude Code or Codex in chats, and each chat works beside a
*stage*: a browser, terminals, code and diff views and a map of the
project, which both you and the agent can see and use.

Cox talks to agents over the
[Agent Client Protocol](https://agentclientprotocol.com) (ACP), so it
works with any agent that speaks ACP.

Cox runs on macOS and Linux. It is in early, active development: there
are no releases yet, and things change often.

## What it does

- **Projects and chats.** A project is a folder. Each chat runs its own
  agent in the project's folder, and keeps running in a background
  process, `coxd`, after the window closes. Closed chats can be resumed.
- **The stage.** Chromium (through CEF) renders web pages next to the
  chat. The agent reads and drives the same pages through the
  [Chrome DevTools MCP server](https://www.npmjs.com/package/chrome-devtools-mcp),
  and you see its pointer as it works. The stage also holds terminals,
  code with syntax colors and language-server support, a file tree, the
  project's changes, an Android emulator, and windows of other Mac apps.
- **Goals.** Give a chat a command that has to pass, such as
  `cargo xtask ci`, or a goal in words that another agent judges. The
  chat keeps working until the goal is met.
- **Sketches.** Try an idea out in a Git worktree or an empty folder,
  apart from the project, then keep the result, rewrite it, or throw it
  away.
- **Triggers and schedules.** Run checks or agents when a turn ends, when
  a goal is met, and on other events, or open a chat on a schedule.
- **A wiki.** Agents keep what they learn in Markdown pages (an Obsidian
  vault works), and keep the sources they read. Cox can translate what
  you read into your language.
- **On the phone.** Follow and answer your chats from your phone, over
  Tailscale.

## Building and installing

Cox is built from source. You need the Rust toolchain
([rustup](https://rustup.rs) installs the version `rust-toolchain.toml`
pins) and Node.js, for `npx`, which starts most agents and the browser
tools.

- **macOS**: the Command Line Tools, plus `cmake` and `ninja`
  (`brew install cmake ninja`).
- **Linux**: the system libraries gpui links (Debian and Ubuntu names):

  ```sh
  sudo apt-get install libvulkan-dev libwayland-dev libx11-xcb-dev libxkbcommon-dev libxkbcommon-x11-dev
  ```

The first build downloads Chromium Embedded Framework (about 200 MB).

```sh
git clone https://github.com/tennashi/cox
cd cox
cargo xtask install
```

This installs Cox to `~/Applications/Cox.app` on macOS, or to
`~/.local/bin/cox` on Linux, and has `coxd` start when you log in.

To update Cox, pull the changes and run `cargo xtask install` again, then
restart the window. `coxd` moves to the new version the next time a
window opens while no chat is at work.

To stop `coxd` starting at login:

- macOS: run `launchctl bootout gui/$(id -u)/com.github.tennashi.cox.coxd`,
  then remove `~/Library/LaunchAgents/com.github.tennashi.cox.coxd.plist`.
- Linux: run `systemctl --user disable coxd.service`.

## Getting started

Tell Cox which agent to run. List your agents in `agents.json`, in Cox's
data folder (`~/Library/Application Support/Cox` on macOS,
`~/.local/share/cox` on Linux):

```json
[
  {
    "name": "Claude",
    "command": [
      "CLAUDE_CODE_ENABLE_TODO_TOOLS=1",
      "CLAUDE_CODE_THINKING_DISPLAY=summarized",
      "npx", "-y", "@agentclientprotocol/claude-agent-acp"
    ]
  }
]
```

and name the one new chats use in `settings.json`, in the same folder:

```json
{"agent": "Claude"}
```

You can also add and test agents from the settings (Cmd-, or Ctrl-,), or
pass the agent's command when you start Cox:

```sh
cox npx -y @agentclientprotocol/claude-agent-acp
```

Then open Cox, press **+** at the foot of the rail to make a project, and
start talking.

See [docs/configuration.md](docs/configuration.md) for every file you
can write to set Cox up, and [CONTRIBUTING.md](CONTRIBUTING.md) to work
on Cox itself.

## License

[MIT](LICENSE)
