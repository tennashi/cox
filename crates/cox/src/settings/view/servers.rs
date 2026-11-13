//! The language servers on the settings screen: Cox's, then those the
//! human names in their own `servers.json`, where Cox keeps what lasts, a
//! row each, with the languages it knows, its command on one line, as a
//! shell takes it (see [`crate::agents::line`]), and where its program is
//! (see [`Found`]).
//!
//! Enter, or the keyboard leaving the row, keeps a command changed: one of
//! Cox's then runs it in place of what Cox downloads, until "Cox's" takes
//! the human's back. × turns one of Cox's off, which "Turn on" undoes, or
//! takes one of the human's out of the file. "Add server" adds one of their
//! own, by its name, its languages and its command. A project's own
//! `servers.json` goes over all of these, for its code alone.
//!
//! The file stays what Cox goes by: what the section keeps it writes to it
//! at once, leaving what else it gives of each server, as its settings,
//! and it reads it again as it changes (see [`ServersSection::reread`]).

use cox_editor::lsp::{Server, Servers};
use gpui::{App, Context, Entity, Focusable, Subscription, Window, div, prelude::*, px, rgb};

use super::words::{Part, Words};
use crate::agents::line;
use crate::stage::servers::Found;
use crate::stage::servers::listing::{self, Listing, Named};
use crate::ui::button::{INPUT_HEIGHT, Kind, button};
use crate::ui::icons::{Icon, icon};
use crate::ui::input::{Blurred, LineInput, Submitted};
use crate::ui::theme::{ACCENT, ASKING, BORDER, DIM, FAILED, FAINT, FOREGROUND, HOVER, LINK};
use crate::ui::tip;
use crate::vim::hints::Hinted as _;

/// The language servers Cox knows, as rows to change.
pub struct ServersSection {
    rows: Vec<Row>,
    /// Why the human's `servers.json` cannot be read, when it cannot.
    unreadable: Option<String>,
    /// What tells the next row from the others.
    next_key: u64,
    words: Words,
}

/// A server as the section shows it, or one the human adds, not kept yet.
struct Row {
    key: u64,
    /// Its name, once it is kept; `None` for one being added.
    name: Option<String>,
    /// The inputs of the name and the languages of one being added.
    adding: Option<(Entity<LineInput>, Entity<LineInput>)>,
    /// The languages it knows.
    languages: Vec<String>,
    command: Entity<LineInput>,
    /// Its command, as kept, as the input shows it.
    saved: String,
    /// Whether it is one of Cox's.
    of_cox: bool,
    /// What the human's file says of it.
    named: Named,
    /// Where its program is, unless it is off.
    found: Option<Found>,
    /// What the line under the row says of what the human did, and in
    /// which color.
    told: Option<(String, u32)>,
    _subscriptions: Vec<Subscription>,
}

impl Row {
    /// Its command, as the input holds it.
    fn text(&self, cx: &App) -> String {
        self.command.read(cx).text().trim().to_owned()
    }

    /// Whether the inputs hold what is not kept.
    fn changed(&self, cx: &App) -> bool {
        self.name.is_none() || self.text(cx) != self.saved
    }
}

/// A server as the files have it, for a row.
struct Shown {
    name: String,
    languages: Vec<String>,
    command: Vec<String>,
    of_cox: bool,
    named: Named,
    found: Option<Found>,
}

/// The servers to show, as Cox's and the human's `servers.json` have them,
/// leaving any project's out: Cox's, in their order, then the human's own.
fn shown(listing: &Listing) -> Vec<Shown> {
    let user = listing::file().ok();
    let (servers, _) = Servers::of_files(user.as_deref());
    let built_in = Servers::built_in();
    let of = |name: &str, servers: &Servers| -> Option<Server> {
        servers
            .all()
            .iter()
            .find(|server| server.name == name)
            .cloned()
    };
    let cox = built_in.all().iter().map(|server| server.name.clone());
    let own = listing
        .names()
        .into_iter()
        .filter(|name| of(name, &built_in).is_none());
    cox.chain(own)
        .filter_map(|name| {
            let named = listing.named(&name);
            let of_cox = of(&name, &built_in).is_some();
            let server = of(&name, &servers).or_else(|| of(&name, &built_in))?;
            let found = (named != Named::Off).then(|| Found::of(&server));
            Some(Shown {
                name,
                languages: server.languages,
                command: server.command,
                of_cox,
                named,
                found,
            })
        })
        .collect()
}

impl ServersSection {
    /// The section, with the servers as the files have them now, writing
    /// what it says in `words`.
    pub fn new(words: Words, cx: &mut Context<Self>) -> Self {
        let mut section = Self {
            rows: Vec::new(),
            unreadable: None,
            next_key: 0,
            words,
        };
        section.reread(cx);
        section
    }

    /// Reads the human's `servers.json` again, as it changed, showing each
    /// server as it has it now; a row whose command is changed and not
    /// kept keeps what it holds, and a row being added stays.
    pub fn reread(&mut self, cx: &mut Context<Self>) {
        let listing = match Listing::read() {
            Ok(listing) => {
                self.unreadable = None;
                listing
            }
            Err(error) => {
                self.unreadable = Some(format!("{error:#}"));
                Listing::default()
            }
        };
        let mut rows = Vec::new();
        for shown in shown(&listing) {
            let command = line::joined(&shown.command);
            let at = self
                .rows
                .iter()
                .position(|row| row.name.as_deref() == Some(&shown.name));
            let mut row = match at {
                Some(at) => self.rows.remove(at),
                None => self.row(Some(shown.name.clone()), &command, cx),
            };
            if !row.changed(cx) {
                row.command
                    .update(cx, |input, cx| input.set_text(&command, cx));
            }
            row.saved = command;
            row.languages = shown.languages;
            row.of_cox = shown.of_cox;
            row.named = shown.named;
            row.found = shown.found;
            rows.push(row);
        }
        rows.extend(self.rows.drain(..).filter(|row| row.name.is_none()));
        self.rows = rows;
        cx.notify();
    }

    /// Keeps each row whose inputs hold what is not kept, as when the
    /// screen closes.
    pub fn save_all(&mut self, cx: &mut Context<Self>) {
        let changed: Vec<u64> = self
            .rows
            .iter()
            .filter(|row| row.name.is_some() && row.changed(cx))
            .map(|row| row.key)
            .collect();
        for key in changed {
            self.save(key, cx);
        }
    }

    /// A row of the server named `name`, running `command`, as one line,
    /// or, with no name, one for the human to add.
    fn row(&mut self, name: Option<String>, command: &str, cx: &mut Context<Self>) -> Row {
        let key = self.next_key;
        self.next_key += 1;
        let line_input = |text: &str, placeholder: &str, cx: &mut Context<Self>| {
            let (text, placeholder) = (text.to_owned(), placeholder.to_owned());
            cx.new(|cx| {
                let mut input = LineInput::new(cx);
                input.set_text(&text, cx);
                input.set_placeholder(&placeholder, cx);
                input
            })
        };
        let command_input = line_input(command, "program arguments…", cx);
        let adding = name
            .is_none()
            .then(|| (line_input("", "Name", cx), line_input("", "Languages", cx)));
        let mut inputs = vec![command_input.clone()];
        if let Some((name, languages)) = &adding {
            inputs.extend([name.clone(), languages.clone()]);
        }
        let mut subscriptions = Vec::new();
        for input in &inputs {
            subscriptions.push(cx.subscribe(input, move |section, _, Submitted, cx| {
                section.save(key, cx);
            }));
            subscriptions.push(cx.subscribe(input, move |section, _, Blurred, cx| {
                section.keep_typed(key, cx);
            }));
        }
        Row {
            key,
            name,
            adding,
            languages: Vec::new(),
            command: command_input,
            saved: command.to_owned(),
            of_cox: false,
            named: Named::No,
            found: None,
            told: None,
            _subscriptions: subscriptions,
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

    /// Keeps the row of `key` when it holds what is not kept, and all it
    /// needs, as when the keyboard leaves it: a row being added, half
    /// written, waits for the rest.
    fn keep_typed(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let whole = row.adding.as_ref().is_none_or(|(name, languages)| {
            !name.read(cx).text().trim().is_empty() && !languages.read(cx).text().trim().is_empty()
        });
        if row.changed(cx) && whole && !row.text(cx).is_empty() {
            self.save(key, cx);
        }
    }

    /// Changes the human's `servers.json` as `change` does, then shows the
    /// servers as it has them, saying `told` under the row of `key`.
    fn change(
        &mut self,
        key: u64,
        told: &str,
        change: impl FnOnce(&mut Listing),
        cx: &mut Context<Self>,
    ) {
        let kept = Listing::read().and_then(|mut listing| {
            change(&mut listing);
            listing.keep()
        });
        if let Err(error) = kept {
            return self.tell(key, format!("{error:#}"), FAILED, cx);
        }
        self.reread(cx);
        self.tell(key, told, ACCENT, cx);
    }

    /// Keeps the row of `key` in the human's `servers.json`, as its inputs
    /// say.
    fn save(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(row) = self.rows.iter().find(|row| row.key == key) else {
            return;
        };
        let command = match line::split(&row.text(cx)) {
            Ok(command) if !command.is_empty() => command,
            Ok(_) => return self.tell(key, "Give the program the server runs.", FAILED, cx),
            Err(why) => return self.tell(key, why, FAILED, cx),
        };
        let Some((name, languages)) = row.adding.as_ref() else {
            let name = row.name.clone().unwrap_or_default();
            return self.change(
                key,
                "Saved.",
                |listing| listing.set(&name, &command, None),
                cx,
            );
        };
        let name = name.read(cx).text().trim().to_owned();
        let languages: Vec<String> = languages
            .read(cx)
            .text()
            .split([',', ' '])
            .filter(|language| !language.is_empty())
            .map(str::to_owned)
            .collect();
        if name.is_empty() {
            return self.tell(key, "Name the server.", FAILED, cx);
        }
        if languages.is_empty() {
            return self.tell(key, "Give the languages it knows.", FAILED, cx);
        }
        if self
            .rows
            .iter()
            .any(|row| row.name.as_deref() == Some(&name))
        {
            return self.tell(
                key,
                format!("A server is named {name} already."),
                FAILED,
                cx,
            );
        }
        // The row is the server's from now on.
        if let Some(row) = self.at(key) {
            row.name = Some(name.clone());
            row.adding = None;
        }
        self.change(
            key,
            "Saved.",
            |listing| listing.set(&name, &command, Some(&languages)),
            cx,
        );
    }

    /// Turns the server of the row of `key` off, if it is Cox's, or takes
    /// it out of the human's `servers.json`, if it is theirs; a row being
    /// added goes.
    fn remove(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(at) = self.rows.iter().position(|row| row.key == key) else {
            return;
        };
        let row = &self.rows[at];
        let Some(name) = row.name.clone() else {
            self.rows.remove(at);
            return cx.notify();
        };
        if row.of_cox {
            self.change(key, "Turned off.", |listing| listing.turn_off(&name), cx);
        } else {
            self.change(key, "", |listing| listing.forget(&name), cx);
        }
    }

    /// Turns the server of the row of `key`, one of Cox's, on again.
    fn turn_on(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(name) = self.at(key).and_then(|row| row.name.clone()) else {
            return;
        };
        self.change(key, "Turned on.", |listing| listing.forget(&name), cx);
    }

    /// Has the server of the row of `key`, one of Cox's, run as Cox runs
    /// it again, in place of the human's command.
    fn as_coxs(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(name) = self.at(key).and_then(|row| row.name.clone()) else {
            return;
        };
        // The input takes Cox's command as the file is read again.
        if let Some(row) = self.at(key) {
            row.saved = String::new();
            row.command.update(cx, |input, cx| input.set_text("", cx));
        }
        self.change(
            key,
            "Back to Cox's.",
            |listing| listing.unset_command(&name),
            cx,
        );
    }

    /// Adds an empty row for one of the human's, with the keyboard in it.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.row(None, "", cx);
        if let Some((name, _)) = &row.adding {
            window.focus(&name.focus_handle(cx));
        }
        self.rows.push(row);
        cx.notify();
    }

    /// What the section shows, a line each, for `cox --inspect state`.
    #[must_use]
    pub fn described(&self, cx: &App) -> Vec<String> {
        let mut lines: Vec<String> = self
            .rows
            .iter()
            .map(|row| {
                let name = row.name.clone().unwrap_or_else(|| "(new)".to_owned());
                let mut line = format!(
                    "Server {name} ({}): {}; {}",
                    row.languages.join(", "),
                    row.text(cx),
                    said(row).0
                );
                if row.changed(cx) {
                    line.push_str(" (not kept)");
                }
                if let Some((told, _)) = row.told.as_ref().filter(|(told, _)| !told.is_empty()) {
                    line.push_str(" — ");
                    line.push_str(told);
                }
                line
            })
            .collect();
        if let Some(why) = &self.unreadable {
            lines.push(format!("servers.json cannot be read: {why}"));
        }
        lines
    }

    fn render_row(row: &Row, words: &mut Part, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let key = row.key;
        let changed = row.changed(cx);
        let off = row.named == Named::Off;
        let who = render_who(row);
        let small = |id: &'static str, label: &'static str, hint: &'static str| {
            button((id, key), label, Kind::Plain)
                .h(px(INPUT_HEIGHT))
                .tooltip(tip(hint))
        };
        let side = if off {
            Some(
                small(
                    "server-on",
                    "Turn on",
                    "Start it again for the code it knows",
                )
                .on_hint_click(cx.listener(move |section, _, _, cx| section.turn_on(key, cx)))
                .into_any_element(),
            )
        } else if row.of_cox && matches!(row.named, Named::Given(Some(_))) {
            Some(
                small(
                    "server-coxs",
                    "Cox's",
                    "Run it as Cox does, in place of your command",
                )
                .on_hint_click(cx.listener(move |section, _, _, cx| section.as_coxs(key, cx)))
                .into_any_element(),
            )
        } else {
            None
        };
        let remove = (!off).then(|| {
            div()
                .id(("server-remove", key))
                .debug_selector(move || format!("server-remove-{key}"))
                .flex_shrink_0()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(HOVER)))
                .child(icon(Icon::Close, 13., FAINT))
                .tooltip(tip(if row.of_cox {
                    "Turn it off"
                } else {
                    "Take it out of servers.json"
                }))
                .on_hint_click(cx.listener(move |section, _, _, cx| section.remove(key, cx)))
        });
        let (found, color) = said(row);
        let told = row
            .told
            .as_ref()
            .filter(|(told, _)| !told.is_empty())
            .map(|(told, color)| words.note(told, *color).pl(px(158.)));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(who)
                    .when(!off, |line| {
                        line.child(framed(&row.command, changed).flex_1().min_w_0())
                    })
                    .when(off, |line| line.child(div().flex_1()))
                    .children(side)
                    .children(remove),
            )
            .child(words.note(&found, color).pl(px(158.)))
            .children(told)
    }
}

/// `input` in its frame, marked when it holds what is not kept.
fn framed(input: &Entity<LineInput>, changed: bool) -> gpui::Div {
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
}

/// The name of the server of `row` and the languages it knows, or the
/// inputs of them for one being added.
fn render_who(row: &Row) -> gpui::Div {
    let column = div().w(px(150.)).flex_shrink_0().flex().flex_col();
    match &row.adding {
        Some((name, languages)) => column
            .gap_1()
            .child(framed(name, true))
            .child(framed(languages, true)),
        None => column
            .pt(px(6.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(if row.named == Named::Off {
                        FAINT
                    } else {
                        FOREGROUND
                    }))
                    .child(row.name.clone().unwrap_or_default()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(FAINT))
                    .truncate()
                    .child(row.languages.join(", ")),
            ),
    }
}

/// What the line under `row` says of where its program is, and in which
/// color.
fn said(row: &Row) -> (String, u32) {
    let yours = row.of_cox && matches!(row.named, Named::Given(Some(_)));
    let found = match &row.found {
        _ if row.name.is_none() => {
            return (
                "Its name, the languages it knows, as LSP names them, and its command.".to_owned(),
                FAINT,
            );
        }
        None => {
            return (
                "Off: Cox starts no server of its own for its languages.".to_owned(),
                FAINT,
            );
        }
        Some(Found::Downloaded(version)) => (format!("Downloaded, version {version}."), DIM),
        Some(Found::ToDownload(version)) => (
            format!("Cox downloads version {version} the first time a code tab needs it."),
            DIM,
        ),
        Some(Found::At(path)) => (format!("Runs {}.", path.display()), DIM),
        Some(Found::Missing(program)) => (
            format!("{program} is not on PATH: install it, or give a command that runs it."),
            ASKING,
        ),
    };
    if yours {
        (
            format!("Your command, in place of Cox's. {}", found.0),
            found.1,
        )
    } else {
        found
    }
}

impl Render for ServersSection {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut words = self.words.part();
        // The words are numbered as they are written, so the heading and
        // the lines over the rows go first.
        let heading = words.heading("LANGUAGE SERVERS");
        let about = words.about(
            "What the code tabs ask what the code means, each with the command that runs \
                 it. A project's own servers.json goes over these for its code.",
        );
        let unreadable = self.unreadable.as_ref().map(|why| {
            words.note(
                &format!("servers.json cannot be read, so it is left as it is: {why}"),
                FAILED,
            )
        });
        let rows: Vec<_> = self
            .rows
            .iter()
            .map(|row| Self::render_row(row, &mut words, cx))
            .collect();
        let hint = words.note(
            "A command is kept on Enter or as you leave it, for code tabs opened after.",
            FAINT,
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading)
            .child(about)
            .children(unreadable)
            .child(div().mt_1().flex().flex_col().gap_3().children(rows))
            .when(self.unreadable.is_none(), |section| {
                section.child(
                    div()
                        .mt_1()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            button(
                                "add-server",
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(icon(Icon::Plus, 14., DIM))
                                    .child("Add server"),
                                Kind::Plain,
                            )
                            .h(px(INPUT_HEIGHT))
                            .on_hint_click(
                                cx.listener(|section, _, window, cx| section.add(window, cx)),
                            ),
                        )
                        .child(hint),
                )
            })
    }
}
