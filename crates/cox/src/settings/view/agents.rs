//! The agents on the settings screen: those the human lists in
//! `agents.json`, a row each, with its name and its command on one line,
//! as a shell takes it (see [`coxd::agents::line`]), which Enter keeps,
//! as the keyboard leaving the row does once it has both.
//! A dot marks the agent a new chat talks to when the command line names
//! none, as `settings.json` names it, and a click on another's dot names
//! that one. "Test" starts the agent and asks it to open a session (see
//! [`coxd::agents::check`]), × takes it out of the list, and "Add agent"
//! adds one of the ACP Registry, as the registry runs it, for the human to
//! change, or one of their own.
//!
//! The files stay what Cox goes by: what the section keeps it writes to
//! them at once, leaving what else they hold, and it reads them again as
//! they change (see [`AgentsSection::reread`]).

use gpui::{
    App, Context, Entity, Focusable, FontWeight, SharedString, Subscription, Window, div,
    prelude::*, px, rgb,
};

use super::words::{Part, Words};
use crate::settings::window::AppAgents;
use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{Blurred, LineInput, Submitted};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;
use coxd::agents::Known;
use coxd::agents::check::{self, Checked};
use coxd::agents::line;
use coxd::agents::listing::Listing;
use coxd::theme::{
    ACCENT, ASKING, BORDER, BORDER_STRONG, DIM, DISABLED, FAILED, FAINT, FOREGROUND, HOVER, LINK,
    RAISED,
};

/// The agents `agents.json` lists, as rows to change.
pub struct AgentsSection {
    rows: Vec<Row>,
    /// What `agents.json` listed as the rows were made or last kept, to
    /// tell a change of the human's to the file from Cox's own.
    listed: Option<Vec<(String, Vec<String>)>>,
    /// Why `agents.json` cannot be read, when it cannot.
    unreadable: Option<String>,
    /// The agent `settings.json` names, by its name.
    default: Option<String>,
    /// Why `settings.json` does not name the agent the human chose, when
    /// Cox could not keep it.
    not_named: Option<String>,
    /// Whether the list of agents to add is open.
    adding: bool,
    /// What tells the next row from the others.
    next_key: u64,
    words: Words,
}

/// An agent `agents.json` lists, or one the human adds, not kept yet.
struct Row {
    key: u64,
    /// Its name as `agents.json` keeps it, if it keeps it.
    kept: Option<String>,
    /// Its name and its command, as kept, as the inputs show them.
    saved: (String, String),
    name: Entity<LineInput>,
    command: Entity<LineInput>,
    /// What the line under the row says, and in which color.
    told: Option<(String, u32)>,
    /// Whether "Test" is trying it out.
    testing: bool,
    _subscriptions: [Subscription; 4],
}

impl Row {
    /// Its name and command, as the inputs hold them.
    fn texts(&self, cx: &App) -> (String, String) {
        (
            self.name.read(cx).text().trim().to_owned(),
            self.command.read(cx).text().trim().to_owned(),
        )
    }

    /// Whether the inputs hold what is not kept.
    fn changed(&self, cx: &App) -> bool {
        self.kept.is_none() || self.texts(cx) != self.saved
    }
}

impl AgentsSection {
    /// The section, with the agents `agents.json` lists now, writing what
    /// it says in `words`.
    pub fn new(words: Words, cx: &mut Context<Self>) -> Self {
        let mut section = Self {
            rows: Vec::new(),
            listed: None,
            unreadable: None,
            default: None,
            not_named: None,
            adding: false,
            next_key: 0,
            words,
        };
        section.reread(cx);
        section
    }

    /// Reads `agents.json` and `settings.json` again, as they changed, and
    /// makes the rows anew when another program changed the agents, as
    /// when the human edits the file.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        self.default = coxd::settings::agent();
        match Listing::read() {
            Ok(listing) => {
                self.unreadable = None;
                let listed = listing.agents();
                if self.listed.as_ref() != Some(&listed) {
                    self.rows = listed
                        .iter()
                        .map(|(name, command)| {
                            self.row(Some(name.clone()), name, &line::joined(command), cx)
                        })
                        .collect();
                    self.listed = Some(listed);
                }
            }
            Err(error) => {
                self.unreadable = Some(format!("{error:#}"));
                self.rows.clear();
                self.listed = None;
            }
        }
        cx.notify();
    }

    /// Keeps each row whose inputs hold what is not kept, as when the
    /// screen closes.
    pub fn save_all(&mut self, cx: &mut Context<Self>) {
        let changed: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| row.changed(cx) && row.texts(cx) != (String::new(), String::new()))
            .map(|row| row.key)
            .collect();
        for key in changed {
            self.save(key, cx);
        }
    }

    /// A row of the agent named `name` running `command`, as one line,
    /// kept as `kept` if it is.
    fn row(
        &mut self,
        kept: Option<String>,
        name: &str,
        command: &str,
        cx: &mut Context<Self>,
    ) -> Row {
        let key = self.next_key;
        self.next_key += 1;
        let name_input = cx.new(|cx| {
            let mut input = LineInput::new(cx);
            input.set_text(name, cx);
            input.set_placeholder("Name", cx);
            input
        });
        let command_input = cx.new(|cx| {
            let mut input = LineInput::wrapping(cx);
            input.set_text(command, cx);
            input.set_placeholder("NAME=value program arguments…", cx);
            input
        });
        let [name_submitted, command_submitted] = [&name_input, &command_input].map(|input| {
            cx.subscribe(input, move |section, _, Submitted, cx| {
                section.save(key, cx);
            })
        });
        let [name_left, command_left] = [&name_input, &command_input].map(|input| {
            cx.subscribe(input, move |section, _, Blurred, cx| {
                section.keep_typed(key, cx);
            })
        });
        Row {
            key,
            kept,
            saved: (name.to_owned(), command.to_owned()),
            name: name_input,
            command: command_input,
            told: None,
            testing: false,
            _subscriptions: [name_submitted, command_submitted, name_left, command_left],
        }
    }

    fn at(&mut self, key: u64) -> Option<&mut Row> {
        self.rows.iter_mut().find(|row| row.key == key)
    }

    /// Says `told` under the row of `key`, in `color`.
    fn tell(&mut self, key: u64, told: impl Into<String>, color: u32, cx: &mut Context<Self>) {
        if let Some(row) = self.at(key) {
            row.told = Some((told.into(), color));
        }
        cx.notify();
    }

    /// Keeps the row of `key` when its inputs hold what is not kept, and
    /// both a name and a command, as when the keyboard leaves one of them:
    /// a row half written, as when the human goes from its name to its
    /// command, waits for the rest.
    fn keep_typed(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let (name, command) = row.texts(cx);
        if row.changed(cx) && !name.is_empty() && !command.is_empty() {
            self.save(key, cx);
        }
    }

    /// Keeps the row of `key` in `agents.json`, as its inputs say.
    fn save(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let (name, text) = row.texts(cx);
        let was = row.kept.clone();
        let command = match line::split(&text) {
            Ok(command) => command,
            Err(why) => return self.tell(key, why, FAILED, cx),
        };
        if name.is_empty() {
            return self.tell(key, "Name the agent.", FAILED, cx);
        }
        if command.iter().all(|word| is_assignment(word)) {
            return self.tell(key, "Give the program the agent runs.", FAILED, cx);
        }
        let kept = Listing::read().and_then(|mut listing| {
            listing.set(was.as_deref(), &name, &command)?;
            listing.keep()?;
            Ok(listing)
        });
        let listing = match kept {
            Ok(listing) => listing,
            Err(error) => return self.tell(key, format!("{error:#}"), FAILED, cx),
        };
        self.listed = Some(listing.agents());
        // The settings go on naming the agent renamed.
        if was.is_some() && self.default == was && was.as_deref() != Some(&name) {
            self.set_default(Some(&name));
        }
        // A row kept for the first time, of one of the human's, comes to
        // be the agent a new chat talks to when none is named.
        if was.is_none() && self.default.is_none() {
            self.set_default(Some(&name));
        }
        let shown = line::joined(&command);
        if let Some(row) = self.at(key) {
            row.kept = Some(name.clone());
            row.command
                .update(cx, |input, cx| input.set_text(&shown, cx));
            row.saved = (name, shown);
            row.told = Some(("Saved.".to_owned(), ACCENT));
        }
        cx.notify();
    }

    /// Names `name` in `settings.json` as the agent a new chat talks to
    /// when the command line names none, or names none, keeping it.
    fn set_default(&mut self, name: Option<&str>) {
        match coxd::settings::keep_agent(name) {
            Ok(()) => {
                self.default = name.map(str::to_owned);
                self.not_named = None;
            }
            Err(error) => {
                tracing::error!("the agent a new chat talks to is not kept: {error:#}");
                self.not_named = Some(format!("{error:#}"));
            }
        }
    }

    /// Makes the agent of the row of `key` the one a new chat talks to.
    fn make_default(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(name) = self.at(key).and_then(|row| row.kept.clone()) else {
            return self.tell(key, "Keep it first, with Enter.", ASKING, cx);
        };
        self.set_default(Some(&name));
        cx.notify();
    }

    /// Takes the agent of the row of `key` out of `agents.json`.
    fn remove(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(at) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        if let Some(name) = self.rows[at].kept.clone() {
            let kept = Listing::read().and_then(|mut listing| {
                listing.remove(&name);
                listing.keep()?;
                Ok(listing)
            });
            match kept {
                Ok(listing) => self.listed = Some(listing.agents()),
                Err(error) => return self.tell(key, format!("{error:#}"), FAILED, cx),
            }
            if self.default.as_deref() == Some(&name) {
                self.set_default(None);
            }
        }
        self.rows.remove(at);
        cx.notify();
    }

    /// Tries the command of the row of `key` out, on a thread of its own,
    /// saying what came of it under the row.
    fn test(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let command = match line::split(&row.texts(cx).1) {
            Ok(command) if !command.is_empty() => command,
            Ok(_) => return self.tell(key, "Give the program the agent runs.", FAILED, cx),
            Err(why) => return self.tell(key, why, FAILED, cx),
        };
        let cwd = match coxd::scratch::folder().map(|folder| folder.join("agent-test")) {
            Ok(cwd) => cwd,
            Err(error) => return self.tell(key, error.to_string(), FAILED, cx),
        };
        if let Some(row) = self.at(key) {
            row.testing = true;
            row.told = Some((
                "Testing: Cox starts the agent and asks it to open a session…".to_owned(),
                DIM,
            ));
        }
        cx.notify();
        let (said, heard) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let checked = std::fs::create_dir_all(&cwd)
                .map_err(|error| Checked::Failed(error.to_string()))
                .map_or_else(|failed| failed, |()| check::check(command, &cwd));
            let _ = said.send(checked);
        });
        cx.spawn(async move |this, cx| {
            let checked = heard
                .await
                .unwrap_or_else(|_| Checked::Failed("the test ended with no answer.".to_owned()));
            this.update(cx, |section, cx| {
                if let Some(row) = section.at(key) {
                    row.testing = false;
                }
                let color = if checked.works() { ACCENT } else { FAILED };
                section.tell(key, checked.said(), color, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Adds a row of `known`, an agent of the registry, kept at once, or,
    /// with none, an empty row for one of the human's, with the keyboard
    /// in it.
    fn add(&mut self, known: Option<Known>, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = false;
        let (name, command) = known.map_or_else(Default::default, |known| {
            (
                known.name,
                line::joined(&coxd::agents::unpinned(&known.command)),
            )
        });
        let row = self.row(None, &name, &command, cx);
        let (key, focused) = (
            row.key,
            if name.is_empty() {
                row.name.clone()
            } else {
                row.command.clone()
            },
        );
        self.rows.push(row);
        if !name.is_empty() {
            self.save(key, cx);
        }
        window.focus(&focused.focus_handle(cx));
        cx.notify();
    }

    /// What the section shows, a line each, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> Vec<String> {
        let mut lines: Vec<String> = self
            .rows
            .iter()
            .map(|row| {
                let (name, command) = row.texts(cx);
                let mark = if row.kept.is_some() && row.kept == self.default {
                    "●"
                } else {
                    "○"
                };
                let mut line = format!("{mark} {name:?}: {command}");
                if row.changed(cx) {
                    line.push_str(" (not kept)");
                }
                if let Some((told, _)) = &row.told {
                    line.push_str(" — ");
                    line.push_str(told);
                }
                line
            })
            .collect();
        if let Some(why) = &self.unreadable {
            lines.push(format!("agents.json cannot be read: {why}"));
        }
        if let Some(why) = &self.not_named {
            lines.push(format!("settings.json does not name the agent: {why}"));
        }
        lines
    }

    fn render_row(
        &self,
        row: &Row,
        words: &mut Part,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = row.key;
        let default = row.kept.is_some() && row.kept == self.default;
        let mark = div()
            .id(("agent-default", key))
            .debug_selector(move || format!("agent-default-{key}"))
            .flex_shrink_0()
            .mt(px(9.))
            .size(px(14.))
            .rounded_full()
            .border_2()
            .border_color(rgb(if default { ACCENT } else { BORDER_STRONG }))
            .when(default, |mark| mark.bg(rgb(ACCENT)))
            .when(!default, |mark| {
                mark.cursor_pointer()
                    .hover(|style| style.border_color(rgb(DIM)))
                    .on_hint_click(
                        cx.listener(move |section, _, _, cx| section.make_default(key, cx)),
                    )
            })
            .tooltip(tip(if default {
                "A new chat talks to it when Cox opens with no agent command"
            } else {
                "Talk to it in a new chat when Cox opens with no agent command"
            }));
        let changed = row.changed(cx);
        let input = |input: &Entity<LineInput>| {
            div()
                .min_h(px(INPUT_HEIGHT))
                .px_2()
                .py(px(4.))
                .flex()
                .items_center()
                .border_1()
                .border_color(rgb(if changed { LINK } else { BORDER }))
                .rounded(px(10.))
                .text_size(px(13.))
                .text_color(rgb(FOREGROUND))
                .child(input.clone())
        };
        let testing = row.testing;
        let test = button(
            ("agent-test", key),
            if testing { "Testing…" } else { "Test" },
            Kind::Plain,
        )
        .h(px(INPUT_HEIGHT))
        .when(testing, |test| {
            test.text_color(rgb(DISABLED)).cursor_default()
        })
        .when(!testing, |test| {
            test.on_hint_click(cx.listener(move |section, _, _, cx| section.test(key, cx)))
        });
        let remove = div()
            .id(("agent-remove", key))
            .debug_selector(move || format!("agent-remove-{key}"))
            .flex_shrink_0()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(10.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(HOVER)))
            .child(icon(Icon::Close, 13., FAINT))
            .tooltip(tip("Take it out of agents.json"))
            .on_hint_click(cx.listener(move |section, _, _, cx| section.remove(key, cx)));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(mark)
                    .child(input(&row.name).w(px(130.)).flex_shrink_0())
                    .child(input(&row.command).flex_1().min_w_0())
                    .child(test)
                    .child(remove),
            )
            .children(
                row.told
                    .as_ref()
                    .map(|(told, color)| words.note(told, *color).pl(px(22.))),
            )
    }

    /// The list of agents to add, under "Add agent" while it is open: one
    /// of the human's own, then those of the registry not listed yet.
    fn render_adding(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let listed: Vec<Known> = cx
            .try_global::<AppAgents>()
            .map(|agents| agents.0.all())
            .unwrap_or_default()
            .into_iter()
            .filter(|known| known.listed && known.download.is_none())
            .filter(|known| {
                self.rows
                    .iter()
                    .all(|row| row.kept.as_ref() != Some(&known.name))
            })
            .collect();
        let choice = |id: SharedString, name: String, said: String| {
            div()
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .px_3()
                .py(px(6.))
                .flex()
                .flex_col()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(HOVER)))
                .child(div().text_size(px(13.)).child(name))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(FAINT))
                        .truncate()
                        .child(said),
                )
        };
        div()
            .p_1()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER))
            .child(
                choice(
                    "add-agent-own".into(),
                    "One of your own".to_owned(),
                    "Its name, and the command that runs it".to_owned(),
                )
                .on_hint_click(cx.listener(|section, _, window, cx| section.add(None, window, cx))),
            )
            .when(!listed.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .pt_2()
                        .pb_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(FAINT))
                        .child(coxd::agents::REGISTRY_GROUP.to_uppercase()),
                )
            })
            .children(listed.into_iter().map(|known| {
                let said = line::joined(&coxd::agents::unpinned(&known.command));
                choice(
                    format!("add-agent-{}", known.id).into(),
                    known.name.clone(),
                    said,
                )
                .on_hint_click(cx.listener(move |section, _, window, cx| {
                    section.add(Some(known.clone()), window, cx);
                }))
            }))
    }
}

/// Whether `word` sets a variable of the environment, as `NAME=value`.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

impl Render for AgentsSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let commanded = cx
            .try_global::<AppAgents>()
            .is_some_and(|agents| agents.0.commanded());
        let mut words = self.words.part();
        // The words are numbered as they are written, so the heading and
        // the lines over the rows go first.
        let heading = words.heading("AGENTS");
        let about = words.about(
            "The agents you run, beside those of the ACP Registry, each with the command \
                 that runs it, NAME=value ahead of the program setting its environment. \
                 The dot marks the one a new chat talks to when Cox opens with no agent \
                 command, as from the Finder or a launcher.",
        );
        let commanded = commanded.then(|| {
            words.note(
                "Cox was started with an agent command, which a new chat talks to in this run.",
                ASKING,
            )
        });
        let unreadable = self.unreadable.as_ref().map(|why| {
            words.note(
                &format!("agents.json cannot be read, so it is left as it is: {why}"),
                FAILED,
            )
        });
        let not_named = self.not_named.as_ref().map(|why| {
            words.note(
                &format!("Cox cannot name the agent a new chat talks to: {why}"),
                FAILED,
            )
        });
        let rows: Vec<_> = self
            .rows
            .iter()
            .map(|row| self.render_row(row, &mut words, cx))
            .collect();
        let hint = words.note(
            "A row is kept on Enter or as you leave it; Test tries it out.",
            FAINT,
        );
        let adding = self.adding;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading)
            .child(about)
            .children(commanded)
            .children(unreadable)
            .child(div().mt_1().flex().flex_col().gap_2().children(rows))
            .children(not_named)
            .when(self.unreadable.is_none(), |section| {
                section
                    .child(
                        div()
                            .mt_1()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                button(
                                    "add-agent",
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(icon(Icon::Plus, 14., DIM))
                                        .child("Add agent"),
                                    Kind::Plain,
                                )
                                .h(px(INPUT_HEIGHT))
                                .on_hint_click(cx.listener(
                                    |section, _, _, cx| {
                                        section.adding = !section.adding;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(hint),
                    )
                    .when(adding, |section| section.child(self.render_adding(cx)))
            })
    }
}
