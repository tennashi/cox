//! A tab of the stage that shows the parts a chat runs with, and how
//! they reach each other, for the human to see what is started before
//! they read the code: the agent's process and Cox's thread for it,
//! speaking ACP; the `cox --mcp` the agent starts and the chat's tool
//! server it reaches over the socket, which asks the stage; the Chrome
//! `DevTools` MCP server the agent starts, the relay it goes through and
//! CEF behind it; `rust-analyzer`, when a tab of Rust code needs it; and
//! the judge of a goal in words. Each part is a box, with what it is,
//! who starts it, and, while the chat's agent runs, how it was started:
//! the agent's command, the socket, the ports. A click on a box opens the
//! code that starts the part, when the project has the file, as it does
//! when the project is Cox.

use std::path::{Path, PathBuf};

use cox_acp::McpServer;
use cox_core::tabs::KeptTab;
use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, ScrollHandle,
    Window, div, prelude::*, px, rgb,
};

use crate::stage::tab::Shows;
use crate::ui::paths::shortened;
use crate::ui::theme::{BACKGROUND, DIM, FOREGROUND, LINK};

/// How wide a part's box is, and the arrow between two.
const BOX_WIDTH: f32 = 180.;
const ARROW_WIDTH: f32 = 84.;

const BORDER: u32 = 0x40_4040;
const HOVER: u32 = 0x2a_2d2e;

/// What a chat runs with, as Cox started its agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    /// The agent's command: the program, then its arguments.
    pub command: Vec<String>,
    /// The socket of the chat's tool server, when it started.
    pub socket: Option<PathBuf>,
    /// The port of the chat's relay, when it started.
    pub relay_port: Option<u16>,
    /// The stage's `DevTools` port, when CEF opened one.
    pub devtools_port: Option<u16>,
    /// The MCP servers the agent was handed, to start.
    pub servers: Vec<McpServer>,
}

/// The human chose a part, to open the code that starts it: `file`, from
/// the project's folder, at the first line that says `find`.
pub struct Opened {
    pub file: PathBuf,
    pub find: &'static str,
}

/// A part a chat runs with.
struct Part {
    name: &'static str,
    /// What it is.
    says: &'static str,
    /// Who starts it, and when.
    started: &'static str,
    /// The code that starts it: the file, from the workspace, and words
    /// of the line to open it at.
    file: &'static str,
    find: &'static str,
}

/// Which way what goes between two parts goes.
#[derive(Clone, Copy)]
enum Way {
    Right,
    Left,
    Both,
}

/// What goes between two parts of a row, and which way.
struct Between {
    says: &'static str,
    way: Way,
}

/// A row of the picture: parts across it, with what goes between them,
/// and when the row is there at all, if not always.
struct Row {
    when: Option<&'static str>,
    parts: Vec<Part>,
    between: Vec<Between>,
}

/// The rows of the picture, as they are for every chat.
fn rows() -> Vec<Row> {
    vec![
        agent_row(),
        tools_row(),
        pages_row(),
        rust_row(),
        judge_row(),
    ]
}

fn part(
    name: &'static str,
    says: &'static str,
    started: &'static str,
    file: &'static str,
    find: &'static str,
) -> Part {
    Part {
        name,
        says,
        started,
        file,
        find,
    }
}

fn between(says: &'static str, way: Way) -> Between {
    Between { says, way }
}

/// The agent, and how what it says reaches the conversation.
fn agent_row() -> Row {
    Row {
        when: None,
        parts: vec![
            part(
                "the agent",
                "The agent's process, which speaks ACP on its standard input and output.",
                "Started by Cox for the chat, in the chat's folder.",
                "crates/cox/src/chat/launcher.rs",
                "fn spawn(",
            ),
            part(
                "agent thread",
                "Cox's thread for the agent: speaks ACP with it, and hands what it says to the \
                 chat.",
                "Started by Cox with the agent, one for each chat.",
                "crates/cox/src/agent.rs",
                "pub fn spawn<",
            ),
            part(
                "conversation",
                "The chat's conversation, on the window's thread, which shows what the agent says \
                 and asks.",
                "Part of the window.",
                "crates/cox/src/chat/conversation.rs",
                "pub fn apply(",
            ),
        ],
        between: vec![
            between("ACP on its stdio", Way::Both),
            between("what the agent says", Way::Right),
        ],
    }
}

/// Cox's own tools: from the bridge the agent starts to the stage.
fn tools_row() -> Row {
    Row {
        when: None,
        parts: vec![
            part(
                "cox --mcp",
                "Cox run again, as the bridge that carries what the agent says to the chat's \
                 tools.",
                "Started by the agent, as its MCP server `cox`.",
                "crates/cox/src/agent/tools.rs",
                "pub fn cox(",
            ),
            part(
                "tools thread",
                "The chat's server of Cox's own tools, which answers on the chat's socket.",
                "Started by Cox with the agent, on a thread of its own.",
                "crates/cox/src/agent/mcp.rs",
                "pub fn start(",
            ),
            part(
                "the stage",
                "The chat's stage, on the window's thread, which carries out what the tools ask.",
                "Part of the window.",
                "crates/cox/src/stage/asked.rs",
                "fn hear_tools(",
            ),
        ],
        between: vec![
            between("the chat's socket", Way::Right),
            between("asks", Way::Right),
        ],
    }
}

/// The pages: from the browser tools the agent starts to CEF.
fn pages_row() -> Row {
    Row {
        when: None,
        parts: vec![
            part(
                "chrome-devtools-mcp",
                "The Chrome DevTools MCP server, with which the agent reads and drives the pages.",
                "Started by the agent, as its MCP server `stage`, through npx.",
                "crates/cox/src/agent/tools.rs",
                "pub fn stage(",
            ),
            part(
                "relay",
                "Cox's relay for the chat: passes everything on, but keeps the agent to the \
                 chat's pages.",
                "Started by Cox with the agent, one for each chat.",
                "crates/cox-browser/src/relay.rs",
                "pub fn start(",
            ),
            part(
                "CEF",
                "Chromium, which draws the stage's pages with no window of its own.",
                "Started by Cox once, for every chat.",
                "crates/cox-browser/src/lib.rs",
                "pub struct Cef",
            ),
        ],
        between: vec![
            between("the relay's port", Way::Right),
            between("the DevTools port", Way::Right),
        ],
    }
}

/// The language server of the project's Rust code.
fn rust_row() -> Row {
    Row {
        when: Some("when a tab shows Rust code"),
        parts: vec![
            part(
                "rust-analyzer",
                "The language server of the project's Rust code, which says what a name means.",
                "Started by Cox when a tab of Rust code first needs it, one for the project.",
                "crates/cox/src/stage/servers.rs",
                "pub fn of(",
            ),
            part(
                "code tabs",
                "The tabs of code, which ask the server what the code means.",
                "Part of the stage.",
                "crates/cox/src/stage/code.rs",
                "pub struct Code {",
            ),
        ],
        between: vec![between("LSP on its stdio", Way::Left)],
    }
}

/// The judge of a goal in words.
fn judge_row() -> Row {
    Row {
        when: Some("when the chat has a goal in words"),
        parts: vec![
            part(
                "the judge",
                "An agent of the chat's command, which judges whether the goal is met.",
                "Started by Cox when the goal is set, in the chat's folder, with none of Cox's \
                 tools.",
                "crates/cox/src/chat/judge.rs",
                "pub fn start(",
            ),
            part(
                "the chat",
                "The chat asks it, each time a turn ends, whether the goal is met now.",
                "Part of the window.",
                "crates/cox/src/chat/judge.rs",
                "pub fn ask(",
            ),
        ],
        between: vec![between("asked at each turn's end", Way::Left)],
    }
}

pub struct Parts {
    /// The chat's folder, where Cox's code is when the project is Cox.
    root: PathBuf,
    /// What the chat runs with, while its agent runs.
    started: Option<Started>,
    rows: Vec<Row>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
}

impl EventEmitter<Opened> for Parts {}

impl Parts {
    /// A tab showing the parts of a chat working in `root`, an absolute
    /// path, which runs with `started`, if its agent runs.
    pub fn new(root: PathBuf, started: Option<Started>, cx: &mut Context<Self>) -> Self {
        Self {
            root,
            started,
            rows: rows(),
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// The chat's agent started, with `started`.
    pub fn runs(&mut self, started: Started, cx: &mut Context<Self>) {
        if self.started.as_ref() != Some(&started) {
            self.started = Some(started);
            cx.notify();
        }
    }

    /// How `part` was started, while the agent runs, as its box says it.
    fn detail(&self, part: &Part) -> Option<String> {
        let started = self.started.as_ref()?;
        let server = |name: &str| {
            started
                .servers
                .iter()
                .find(|server| server.name == name)
                .map(|server| {
                    let mut words = vec![server.command.to_string_lossy().into_owned()];
                    words.extend(server.args.iter().cloned());
                    words.join(" ")
                })
        };
        match part.name {
            "the agent" => Some(started.command.join(" ")),
            "cox --mcp" => started
                .socket
                .as_ref()
                .map(|socket| format!("on {}", shortened(socket))),
            "chrome-devtools-mcp" => server("stage"),
            "relay" => started.relay_port.map(|port| format!("port {port}")),
            "CEF" => started
                .devtools_port
                .map(|port| format!("DevTools on port {port}")),
            _ => None,
        }
    }

    /// The line above the picture.
    fn render_heading(&self) -> AnyElement {
        let said = if self.started.is_some() {
            "What the chat runs with, and how they reach each other. A click on a part opens \
             the code that starts it."
        } else {
            "The chat is closed, so its agent is not started: a line sent starts it again, \
             with these parts. A click on a part opens the code that starts it."
        };
        div()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_color(rgb(DIM))
            .child(said)
            .into_any_element()
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let mut across: Vec<AnyElement> = Vec::new();
        for (at, part) in row.parts.iter().enumerate() {
            if at > 0
                && let Some(between) = row.between.get(at - 1)
            {
                across.push(render_between(between));
            }
            across.push(self.render_part(index * 10 + at, part, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .when_some(row.when, |row, when| {
                row.child(div().text_xs().text_color(rgb(DIM)).child(when))
            })
            .child(div().flex().items_center().children(across))
            .into_any_element()
    }

    /// A part's box: its name, what it is, who starts it, how it was
    /// started while the agent runs, and the file that starts it.
    fn render_part(&self, number: usize, part: &Part, cx: &mut Context<Self>) -> AnyElement {
        let file = PathBuf::from(part.file);
        let has = self.root.join(&file).is_file();
        let find = part.find;
        let name = file.file_name().map_or_else(
            || part.file.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let detail = self.detail(part);
        let mut element = div()
            .id(("part", number))
            .debug_selector(move || format!("part-{number}"))
            .w(px(BOX_WIDTH))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(FOREGROUND))
                    .child(part.name),
            )
            .child(div().text_xs().text_color(rgb(DIM)).child(part.says))
            .child(div().text_xs().text_color(rgb(DIM)).child(part.started))
            .when_some(detail, |element, detail| {
                element.child(
                    div()
                        .text_xs()
                        .text_color(rgb(FOREGROUND))
                        .truncate()
                        .child(detail),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if has { LINK } else { DIM }))
                    .child(name),
            );
        if has {
            element = element
                .cursor_pointer()
                .hover(|style| style.bg(rgb(HOVER)))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(Opened {
                        file: file.clone(),
                        find,
                    });
                }));
        }
        element.into_any_element()
    }
}

/// What goes between two parts: its words over an arrow.
fn render_between(between: &Between) -> AnyElement {
    let (left, right) = match between.way {
        Way::Right => (None, Some("▶")),
        Way::Left => (Some("◀"), None),
        Way::Both => (Some("◀"), Some("▶")),
    };
    div()
        .w(px(ARROW_WIDTH))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap_1()
        .px_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(DIM))
                .text_center()
                .child(between.says),
        )
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .text_xs()
                .text_color(rgb(DIM))
                .children(left)
                .child(div().flex_1().h(px(2.)).bg(rgb(DIM)))
                .children(right),
        )
        .into_any_element()
}

/// The line of the file at `path` that first says `find`, as people
/// number lines, if one does.
#[must_use]
pub fn line_saying(path: &Path, find: &str) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .position(|line| line.contains(find))
        .map(|index| index + 1)
}

impl Shows for Parts {
    fn title(&self) -> String {
        "Parts".to_owned()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Parts
    }
}

impl Focusable for Parts {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Parts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = std::mem::take(&mut self.rows);
        let drawn: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_row(index, row, cx))
            .collect();
        self.rows = rows;
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(self.render_heading())
            .child(
                div()
                    .id("parts")
                    .debug_selector(|| "parts".to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .py_1()
                    .children(drawn),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_names_code_that_exists() {
        // The tests run from the crate's folder, two below the workspace.
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for row in rows() {
            assert_eq!(
                row.between.len() + 1,
                row.parts.len(),
                "{}",
                row.parts[0].name
            );
            for part in row.parts {
                let path = workspace.join(part.file);
                assert!(
                    line_saying(&path, part.find).is_some(),
                    "{} says nothing of {:?}",
                    part.file,
                    part.find
                );
            }
        }
    }

    #[test]
    fn the_line_saying_words_is_found_as_people_number_lines() {
        let path = std::env::temp_dir().join(format!("cox-parts-{}.rs", std::process::id()));
        std::fs::write(&path, "fn a() {}\n\npub fn start() {}\n").unwrap();
        assert_eq!(line_saying(&path, "pub fn start("), Some(3));
        assert_eq!(line_saying(&path, "fn stop("), None);
        std::fs::remove_file(path).unwrap();
    }
}
