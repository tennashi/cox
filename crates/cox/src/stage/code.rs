//! A tab of the stage that shows a file as it is written, such as code,
//! for the human to read beside the conversation. The agent shows it with
//! Cox's tool for that (see [`coxd::agent::mcp`]), at the lines it talks about,
//! which the tab marks and scrolls to; a click on a link to a file opens
//! it too.
//!
//! The tab draws only the lines in sight, so that a long file scrolls as a
//! short one does, each with its number beside it, in colors as its syntax
//! says, when its language is one the editor knows (see
//! [`cox_editor::language`]). It follows the file as it changes (see
//! [`crate::stage::follow`]), and parses it again in the background, keeping its
//! colors until then. The wheel scrolls it down and across, and a drag
//! selects text to copy: a double click selects a word, and a triple click
//! a line.
//!
//! When the editor knows a server for the file's language, the tab asks
//! the project's (see [`crate::stage::servers`]) what the code means. The mouse
//! resting on a name shows what the server says of it, its type and its
//! documentation (see [`hover`]). A click on a name with Command held, or
//! Control off macOS, goes to where the name is defined, and the menu of
//! the right button goes there too, or lists where the name is used (see
//! [`locations`]). Going to a place in another file, the tab shows that file,
//! as a page goes to another: back and forward go between the places the
//! tab went to.
//!
//! - [`file`] is the file shown, with what is worked out of it to draw.
//! - [`text`] draws the lines in sight, and [`bar`] the bar above them.
//! - [`menu`] is the menu of the right button, [`hover`] what the server
//!   says of a name, and [`locations`] the places it found.

mod bar;

mod file;
mod hover;
mod locations;
mod menu;
mod text;

use std::ops::Range;
use std::path::{Path, PathBuf};

use cox_core::entry::Attachment;
use cox_core::tabs::{KeptTab, Lines};
use cox_editor::buffer::{Buffer, Point as Place, is_word, word_around};
use cox_editor::history::History;
use cox_editor::language::Highlight;
use cox_editor::line::Shown;
use cox_editor::lsp::{Client, Encoding, OpenFile, Position};
use cox_editor::syntax::Syntax;

use self::file::File;
use self::text::Text;
/// How code is drawn, by what its syntax says it is, as a tab of code
/// draws it: for another tab that shows code, as "Changes" does.
pub(crate) use self::text::style as syntax_style;
use crate::vim::cursor::{self, Cursor};
use gpui::{
    App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection, Pixels, Point,
    ScrollWheelEvent, ShapedLine, Subscription, Task, Window, actions, div, point, prelude::*, px,
    size,
};

use crate::chat::region::{self, Described};
use crate::stage::follow::{self, Follows};
use crate::stage::servers::{LanguageServer, Servers};
use crate::stage::tab::{self, Shows, ShowsFile};
use crate::ui::selected::{self, Selected};
use crate::ui::{Copy, LinkClicked};
use crate::vim::keymap::Motion;
use crate::vim::search::{self, Find, Search};
use crate::vim::{
    Definition, FindWord, LookUp, Move, MoveAcross, References, ScrollTo, VisualCharacters,
};
use coxd::paths::shortened;
use coxd::storage::Stamp;

actions!(
    code,
    [
        /// Goes back to where the tab was before it went to a place.
        Back,
        /// Goes forward again to where the tab went back from.
        Forward,
        /// Closes the menu, or else what the server says of a name, or
        /// else the list of places.
        Dismiss,
    ]
);

pub(crate) const KEY_CONTEXT: &str = "Code";

/// Room between the text and the tab's edges.
pub(super) const PADDING: f32 = 8.;

/// How many lines a turn of the wheel scrolls.
const WHEEL_LINES: f32 = 3.;

/// Binds the keys of the tab.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-[", Back, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-]", Forward, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-left", Back, Some(KEY_CONTEXT)),
        KeyBinding::new("alt-right", Forward, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

pub struct Code {
    /// The file shown, with what is worked out of it to draw it.
    file: File,
    /// How the tab looks at the file.
    look: Look,
    /// The project's server that knows the language of the file, if the
    /// editor knows one.
    server: Option<Entity<LanguageServer>>,
    /// The folder of the project whose servers the tab asks, when it
    /// shows a file of another language than it did: none for a tab that
    /// asks the one server it was given, if any.
    project: Option<PathBuf>,
    /// The file, open on the server once it is ready.
    document: Option<OpenFile>,
    /// Where the tab was before it went to a place, to go back to, and
    /// where it went back from.
    history: History<Visit>,
    /// The word the mouse rests on, as its line and the bytes of it.
    pointed: Option<(usize, Range<usize>)>,
    /// Whether the key that makes a click go to a definition is held.
    linking: bool,
    /// What the server says of the name the mouse rests on, once it says.
    hover: Option<hover::Hovered>,
    /// The menu of the right button, where it opened in the window, for
    /// the word at the place given.
    menu: Option<(Point<Pixels>, Place)>,
    /// The places the server found last, listed under the code.
    locations: Option<locations::Locations>,
    /// What the tab says of what the human asked, such as that the server
    /// found nothing.
    notice: Option<String>,
    focus_handle: FocusHandle,
    _follow: Task<()>,
    _parse: Task<()>,
    /// Waits for the mouse to rest, then asks the server of the name.
    _hovering: Task<()>,
    /// Hides what the server says of a name once the mouse leaves it.
    _unhovering: Task<()>,
    /// Asks the server what the human asked.
    _asking: Task<()>,
    _server_changes: Option<Subscription>,
}

/// How the tab looks at its file: what it marks and selects, and how far
/// it is scrolled. Another file is looked at afresh.
#[derive(Default)]
struct Look {
    /// The lines the tab was last shown at, if any.
    marked: Option<Lines>,
    /// The line, from 0, to bring into sight once the tab is laid out.
    reveal: Option<usize>,
    /// How far the text is scrolled across and down.
    scroll: Point<Pixels>,
    /// Where the selection started, and where it goes to, if there is one.
    selection: Option<(Place, Place)>,
    /// Whether a drag is selecting.
    selecting: bool,
    /// How the last frame laid the text out.
    laid: Option<Laid>,
    /// The cursor Vim's keys move, once drawn with them on.
    cursor: Option<Cursor>,
    /// Where Vim's visual mode started, and what it selects from there
    /// to the cursor, while it is on.
    visual: Option<(Place, Visual)>,
    /// Which of the cursor and the scroll the next frame keeps to.
    follow: Follow,
    /// What Vim's `/` looked for last, whose matches are marked.
    found: Option<Find>,
}

/// What Vim's visual mode selects from where it started to the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Visual {
    /// The characters, both ends with them (`v`).
    Characters,
    /// The whole lines (`V`).
    Lines,
}

/// What the next frame keeps to, of the cursor of Vim's keys and the
/// scroll, which it brings the other in sight of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Follow {
    /// The cursor moved by the keys: the text scrolls to it.
    Cursor,
    /// The text scrolled: the cursor moves into sight.
    #[default]
    Scroll,
}

impl Look {
    /// Puts the cursor at `cursor`, in `text`, and the end of the
    /// selection of visual mode with it, while it is on.
    fn put_cursor(&mut self, cursor: Cursor, text: &Buffer) {
        self.cursor = Some(cursor);
        let Some((from, visual)) = self.visual else {
            return;
        };
        let at = cursor.place(&text.line(cursor.line));
        let (start, end) = (from.min(at), from.max(at));
        self.selection = Some(match visual {
            Visual::Lines => (Place::new(start.line, 0), Place::new(end.line + 1, 0)),
            // An empty line's character is its line break.
            Visual::Characters => match text.line(end.line)[end.column..].chars().next() {
                Some(c) => (start, Place::new(end.line, end.column + c.len_utf8())),
                None => (start, Place::new(end.line + 1, 0)),
            },
        });
    }

    /// Keeps how a frame laid the text out, as `laid`, scrolled `scroll`,
    /// with the cursor of Vim's keys on `cursor` of `text`, if they are on.
    fn laid_out(
        &mut self,
        laid: Laid,
        scroll: Point<Pixels>,
        cursor: Option<usize>,
        text: &Buffer,
    ) {
        if let Some(line) = cursor {
            self.put_cursor(self.cursor_on(line), text);
        }
        self.follow = Follow::Scroll;
        self.reveal = None;
        self.scroll = scroll;
        self.laid = Some(laid);
    }

    /// The cursor, moved to `line`, at the character it goes to.
    fn cursor_on(&self, line: usize) -> Cursor {
        self.cursor
            .map_or(Cursor::on(line), |cursor| cursor.to_line(line))
    }

    /// Leaves visual mode, with its selection: answers whether it was on.
    fn end_visual(&mut self) -> bool {
        let was = self.visual.take().is_some();
        if was {
            self.selection = None;
        }
        was
    }
}

/// Where the tab was: the file, what it marked and selected there, and
/// how far it was scrolled.
#[derive(Clone, Debug, PartialEq)]
struct Visit {
    path: PathBuf,
    marked: Option<Lines>,
    selection: Option<(Place, Place)>,
    scroll: Point<Pixels>,
}

/// How a frame laid the text out, to find what is under the mouse.
struct Laid {
    /// Where the text shows.
    text: Bounds<Pixels>,
    line_height: Pixels,
    cell_width: Pixels,
    /// The lines in sight, in order.
    rows: Vec<Row>,
}

impl Laid {
    /// Where `range`, bytes of `line`, is drawn, if the line is in sight.
    fn stretch(&self, line: usize, range: &Range<usize>) -> Option<Bounds<Pixels>> {
        let row = self.rows.iter().find(|row| row.line == line)?;
        let x = |column| row.shaped.x_for_index(row.shown.index_of(column));
        Some(Bounds::new(
            row.origin + point(x(range.start), px(0.)),
            size(x(range.end) - x(range.start), self.line_height),
        ))
    }
}

/// A line in sight, as drawn.
struct Row {
    line: usize,
    /// The line's text, at its top left.
    origin: Point<Pixels>,
    shaped: ShapedLine,
    shown: Shown,
}

impl EventEmitter<LinkClicked> for Code {}

impl Code {
    /// A tab showing the file at `path`, an absolute path, which says
    /// `text`, at `lines` if any, which asks `server` what the code means,
    /// if given.
    pub fn new(
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        server: Option<Entity<LanguageServer>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let server_changes = server.as_ref().map(|server| Self::hear(server, cx));
        let mut code = Self {
            file: File::new(path, text),
            look: Look {
                marked: lines,
                reveal: lines.map(|lines| lines.first - 1),
                ..Look::default()
            },
            server,
            project: None,
            document: None,
            history: History::new(),
            pointed: None,
            linking: false,
            hover: None,
            menu: None,
            locations: None,
            notice: None,
            focus_handle: cx.focus_handle(),
            _follow: follow::follow(cx),
            _parse: Task::ready(()),
            _hovering: Task::ready(()),
            _unhovering: Task::ready(()),
            _asking: Task::ready(()),
            _server_changes: server_changes,
        };
        code.parse(cx);
        code.open_on_server(cx);
        code
    }

    /// Hears `server` grow ready, which then gets the file.
    fn hear(server: &Entity<LanguageServer>, cx: &mut Context<Self>) -> Subscription {
        cx.observe(server, |code, _, cx| {
            code.open_on_server(cx);
            cx.notify();
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file.path
    }

    /// The text shown.
    #[must_use]
    pub fn text(&self) -> String {
        self.file.buffer.text()
    }

    /// The lines the tab was last shown at, if any.
    #[must_use]
    pub fn marked(&self) -> Option<Lines> {
        self.look.marked
    }

    /// The lines in sight, from 0, as the last frame drew them.
    #[must_use]
    pub fn in_sight(&self) -> std::ops::Range<usize> {
        let rows = self
            .look
            .laid
            .as_ref()
            .map_or(&[][..], |laid| &laid.rows[..]);
        match (rows.first(), rows.last()) {
            (Some(first), Some(last)) => first.line..last.line + 1,
            _ => 0..0,
        }
    }

    /// The line, from 0, of the cursor Vim's keys move, once drawn with
    /// them on.
    #[must_use]
    pub fn cursor(&self) -> Option<usize> {
        self.look.cursor.map(|cursor| cursor.line)
    }

    /// Where the cursor Vim's keys move is, as its line and the byte of
    /// the line it is at, once drawn with them on.
    #[must_use]
    pub fn cursor_place(&self) -> Option<(usize, usize)> {
        let place = self.cursor_at()?;
        Some((place.line, place.column))
    }

    /// Where the cursor Vim's keys move is in the text.
    fn cursor_at(&self) -> Option<Place> {
        let cursor = self.look.cursor?;
        Some(cursor.place(&self.file.buffer.line(cursor.line)))
    }

    /// What the server says of the name the mouse rests on, as Markdown,
    /// while it shows.
    #[must_use]
    pub fn hover(&self) -> Option<&str> {
        self.hover.as_ref().map(hover::Hovered::markdown)
    }

    /// Where in the window the middle of the character at `column` of
    /// `line`, both from 0, is drawn, as the last frame drew it, if the
    /// line was in sight.
    #[must_use]
    pub fn point_of(&self, line: usize, column: usize) -> Option<Point<Pixels>> {
        let laid = self.look.laid.as_ref()?;
        let at = laid.stretch(line, &(column..column + 1))?;
        Some(at.center())
    }

    /// What the stretches of `line`, from 0, are, as bytes of it, as the
    /// syntax says once parsed.
    #[must_use]
    pub fn highlights(&self, line: usize) -> Vec<(Range<usize>, Highlight)> {
        self.file.highlights(line)
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> String {
        match self.look.selection {
            Some((anchor, head)) => self.file.buffer.slice(anchor.min(head)..anchor.max(head)),
            None => String::new(),
        }
    }

    /// Shows `text`, which the file says now, at `lines` if any, which the
    /// tab scrolls to.
    pub fn show(&mut self, text: &str, lines: Option<Lines>, cx: &mut Context<Self>) {
        // Back goes to where the human was reading.
        if lines.is_some() && lines != self.look.marked {
            self.history.leave(self.visit());
        }
        // The tab looks at the file again, which may have changed since.
        self.file.stamp = None;
        self.file.unread = None;
        self.set_text(text, cx);
        self.look.marked = lines;
        self.look.reveal = lines.map(|lines| lines.first - 1);
        cx.notify();
    }

    fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.file.set_text(text) {
            // What was selected may be gone.
            self.look.selection = None;
            self.look.visual = None;
            self.look.selecting = false;
            self.parse(cx);
            // What the server said of the text before may not hold.
            self.hover = None;
            self.menu = None;
            if let Some(document) = &self.document {
                document.change(text);
            }
        }
    }

    /// Opens the file on the server, once the server is ready, if it is
    /// in the server's language.
    fn open_on_server(&mut self, cx: &mut Context<Self>) {
        if self.document.is_some() {
            return;
        }
        let Some(server) = self.server.clone() else {
            return;
        };
        let server = server.read(cx);
        let Some(client) = server.client() else {
            return;
        };
        let Some(language) = server.language_of(&self.file.path) else {
            return;
        };
        self.document = client.open(&self.file.path, language, &self.file.buffer.text());
    }

    /// The server, once it is ready to be asked.
    fn client<'a>(&self, cx: &'a App) -> Option<&'a Client> {
        self.server.as_ref()?.read(cx).client()
    }

    /// What the server counts a line in.
    fn encoding(&self, cx: &App) -> Encoding {
        self.client(cx)
            .map_or_else(Encoding::default, Client::encoding)
    }

    /// Asks the server `request` of `place` in the file, once the server
    /// is ready and has the file open: what to wait on for its answer.
    fn ask<F>(
        &self,
        place: Place,
        request: impl FnOnce(&Client, &OpenFile, Position) -> F,
        cx: &App,
    ) -> Option<F> {
        let client = self.client(cx)?;
        let document = self.document.as_ref()?;
        let position = client.encoding().position(&self.file.buffer, place);
        Some(request(client, document, position))
    }

    /// Where the tab is now.
    fn visit(&self) -> Visit {
        Visit {
            path: self.file.path.clone(),
            marked: self.look.marked,
            selection: self.look.selection,
            scroll: self.look.scroll,
        }
    }

    /// Shows the file at `path`, an absolute path, from its start, unless
    /// the tab shows it already: answers whether it does now.
    fn open(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if path == self.file.path {
            return true;
        }
        self.file = match File::read(path) {
            Ok(file) => file,
            Err(error) => {
                self.notice = Some(format!("cannot read {}: {error}", shortened(path)));
                return false;
            }
        };
        self.look = Look::default();
        self.hover = None;
        self.menu = None;
        self.document = None;
        self.serve(cx);
        self.parse(cx);
        self.open_on_server(cx);
        true
    }

    /// Asks the project's server of the file's language, when the file is
    /// of another than the server the tab asked knows.
    #[expect(
        clippy::used_underscore_binding,
        reason = "what the server says is only heard: dropping it stops hearing it"
    )]
    fn serve(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let knows = self
            .server
            .as_ref()
            .is_some_and(|server| server.read(cx).language_of(&self.file.path).is_some());
        if knows {
            return;
        }
        self.server = cx
            .try_global::<Servers>()
            .cloned()
            .and_then(|servers| servers.of(&project, &self.file.path, cx));
        self._server_changes = self.server.as_ref().map(|server| Self::hear(server, cx));
    }

    /// Shows the file at `path`, an absolute path, from its start, as the
    /// human chose it: back goes to where the tab was.
    pub fn go_to_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let here = self.visit();
        if self.open(path, cx) && self.visit() != here {
            self.history.leave(here);
        }
        cx.notify();
    }

    /// Goes to `range` of the file at `path`, as the server counts it,
    /// which it selects and marks: back goes to where the tab was.
    fn go_to(&mut self, path: &Path, range: Range<Position>, cx: &mut Context<Self>) {
        let here = self.visit();
        let in_sight = self.in_sight();
        let same_file = path == self.file.path;
        if !self.open(path, cx) {
            return;
        }
        let encoding = self.encoding(cx);
        let start = encoding.point(&self.file.buffer, range.start);
        let end = encoding.point(&self.file.buffer, range.end).max(start);
        self.look.selection = Some((start, end));
        self.look.marked = Some(Lines {
            first: start.line + 1,
            last: end.line + 1,
        });
        // A line in sight stays where it is.
        if !(same_file && in_sight.contains(&start.line)) {
            self.look.reveal = Some(start.line);
        }
        if self.visit() != here {
            self.history.leave(here);
        }
        cx.notify();
    }

    /// Goes to where the tab was at `visit`: answers whether it could.
    fn revisit(&mut self, visit: &Visit, cx: &mut Context<Self>) -> bool {
        if !self.open(&visit.path, cx) {
            return false;
        }
        self.look.marked = visit.marked;
        self.look.selection = visit.selection;
        self.look.scroll = visit.scroll;
        self.look.reveal = None;
        self.look.visual = None;
        self.hover = None;
        self.notice = None;
        cx.notify();
        true
    }

    fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        self.go_back(cx);
    }

    fn forward(&mut self, _: &Forward, _: &mut Window, cx: &mut Context<Self>) {
        self.go_forward(cx);
    }

    /// Goes back to where the tab was before it went to a place.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        let Some(visit) = self.history.back() else {
            return;
        };
        let here = self.visit();
        if self.revisit(&visit, cx) {
            self.history.went_back(here);
        }
    }

    /// Goes forward again to where the tab went back from.
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        let Some(visit) = self.history.forward() else {
            return;
        };
        let here = self.visit();
        if self.revisit(&visit, cx) {
            self.history.went_forward(here);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if !self.look.end_visual() && self.menu.take().is_none() && self.hover.take().is_none() {
            self.locations = None;
        }
        cx.notify();
    }

    /// Parses the text in the background, with what did not change since
    /// it was last parsed, and draws it so once parsed.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the parse is only held: dropping it leaves its syntax unused"
    )]
    fn parse(&mut self, cx: &mut Context<Self>) {
        let Some(language) = self.file.language else {
            return;
        };
        let buffer = self.file.buffer.clone();
        let old = self.file.syntax.clone();
        // A parse of the text before, if not done yet, is left to end.
        self._parse = cx.spawn(async move |code, cx| {
            let syntax = cx
                .background_executor()
                .spawn(async move { Syntax::parse(language, &buffer, old.as_ref()) })
                .await;
            code.update(cx, |code, cx| {
                code.file.syntax = syntax;
                code.file.highlighted.clear();
                cx.notify();
            })
            .ok();
        });
    }

    /// The place in the text under `position`: on the nearest line in
    /// sight, at the nearest character.
    fn place_at(&self, position: Point<Pixels>) -> Option<Place> {
        let laid = self.look.laid.as_ref()?;
        let first = laid.rows.first()?;
        let last = laid.rows.last()?;
        let row = if position.y < first.origin.y {
            first
        } else {
            laid.rows
                .iter()
                .find(|row| position.y < row.origin.y + laid.line_height)
                .unwrap_or(last)
        };
        let index = row.shaped.closest_index_for_x(position.x - row.origin.x);
        Some(Place::new(row.line, row.shown.column_of(index)))
    }

    /// The word under `position`, as its line and the bytes of it, if the
    /// mouse is on one.
    fn word_at(&self, position: Point<Pixels>) -> Option<(usize, Range<usize>)> {
        let laid = self.look.laid.as_ref()?;
        let row = laid.rows.iter().find(|row| {
            row.origin.y <= position.y && position.y < row.origin.y + laid.line_height
        })?;
        if !laid.text.contains(&position) || position.x < row.origin.x {
            return None;
        }
        let index = row.shaped.index_for_x(position.x - row.origin.x)?;
        let line = self.file.buffer.line(row.line);
        let word = word_around(&line, row.shown.column_of(index));
        line[word.clone()]
            .starts_with(is_word)
            .then_some((row.line, word))
    }

    /// The text of the word at `place`.
    fn word_text(&self, place: Place) -> String {
        let line = self.file.buffer.line(place.line);
        line[word_around(&line, place.column.min(line.len()))].to_owned()
    }

    /// Where in the window the bottom left of `column` of `line` is, if the
    /// line is in sight.
    fn below(&self, line: usize, column: usize) -> Option<Point<Pixels>> {
        self.look
            .laid
            .as_ref()?
            .stretch(line, &(column..column))
            .map(|at| at.bottom_left())
    }

    /// Where the name the server says something of is, and the line under
    /// the name a click goes to the definition of, laid out as `laid`.
    fn names(&self, laid: &Laid) -> (Option<Bounds<Pixels>>, Option<Bounds<Pixels>>) {
        let hovered = self
            .hover
            .as_ref()
            .and_then(|hovered| laid.stretch(hovered.line, &hovered.range));
        let link = self
            .pointed
            .as_ref()
            .filter(|_| self.linking)
            .and_then(|(line, word)| laid.stretch(*line, word))
            .map(|at| {
                Bounds::new(
                    at.bottom_left() - point(px(0.), px(2.)),
                    size(at.size.width, px(1.)),
                )
            });
        (hovered, link)
    }

    /// How far the text scrolls, at most, laid out as `laid`.
    fn most_scroll(&self, laid: &Laid) -> Point<Pixels> {
        #[expect(
            clippy::cast_precision_loss,
            reason = "line and character counts are far below what f32 keeps exactly"
        )]
        let (lines, longest) = (self.file.buffer.lines() as f32, self.file.longest as f32);
        // The last line scrolls up to the middle.
        let down = laid.line_height * lines + px(PADDING) - laid.text.size.height / 2.;
        let across = laid.cell_width * longest + px(PADDING * 2.) - laid.text.size.width;
        point(across.max(px(0.)), down.max(px(0.)))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        self.unhover();
        self.notice = None;
        if event.modifiers.secondary()
            && let Some((line, word)) = self.word_at(event.position)
        {
            self.look.selection = None;
            self.go_to_definition(Place::new(line, word.start), cx);
            return;
        }
        let Some(place) = self.place_at(event.position) else {
            return;
        };
        self.look.visual = None;
        let line = self.file.buffer.line(place.line);
        if self.look.cursor.is_some() {
            self.look.cursor = Some(Cursor::at(place, &line));
        }
        self.look.selection = Some(match event.click_count {
            1 => (place, place),
            2 => {
                let word = word_around(&line, place.column);
                (
                    Place::new(place.line, word.start),
                    Place::new(place.line, word.end),
                )
            }
            _ => (Place::new(place.line, 0), Place::new(place.line + 1, 0)),
        });
        self.look.selecting = event.click_count == 1;
        cx.notify();
    }

    /// The right button opens the menu for the word under the mouse.
    fn right_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.unhover();
        self.menu = self
            .word_at(event.position)
            .map(|(line, word)| (event.position, Place::new(line, word.start)));
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.link(event.modifiers.secondary(), cx);
        if event.pressed_button.is_none() {
            let word = self.word_at(event.position);
            self.rest_on(word, cx);
            return;
        }
        if !self.look.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(place) = self.place_at(event.position)
            && let Some((_, head)) = &mut self.look.selection
            && *head != place
        {
            *head = place;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.look.selecting = false;
        // A click with no drag selects nothing.
        if self
            .look
            .selection
            .is_some_and(|(anchor, head)| anchor == head)
        {
            self.look.selection = None;
            cx.notify();
        }
    }

    /// Whether the key that makes a click go to a definition is held.
    fn link(&mut self, linking: bool, cx: &mut Context<Self>) {
        if linking != self.linking {
            self.linking = linking;
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.hover = None;
        self.menu = None;
        let Some(laid) = &self.look.laid else {
            return;
        };
        let delta = event.delta.pixel_delta(laid.line_height * WHEEL_LINES);
        let most = self.most_scroll(laid);
        let scroll = point(
            (self.look.scroll.x - delta.x).clamp(px(0.), most.x),
            (self.look.scroll.y - delta.y).clamp(px(0.), most.y),
        );
        if scroll != self.look.scroll {
            self.look.scroll = scroll;
            cx.notify();
        }
        cx.stop_propagation();
    }

    /// The text selected in the code, with the file and the lines it is
    /// from, as it goes to the agent when handed to the chat.
    fn selected_attachment(&self) -> Attachment {
        let lines = self.look.selection.map(|(anchor, head)| {
            let (start, end) = (anchor.min(head), anchor.max(head));
            // A selection that ends at the start of a line ends on the
            // line before.
            let last = if end.column == 0 && end.line > start.line {
                end.line
            } else {
                end.line + 1
            };
            Lines {
                first: start.line + 1,
                last,
            }
        });
        crate::chat::hand::of_file(&self.file.path, lines, self.selected_text())
    }

    /// What the lines of the code under `over`, a stretch of a region in
    /// the window's points, are, as the tab last drew them.
    fn described(&self, over: Bounds<Pixels>) -> Option<Described> {
        let laid = self.look.laid.as_ref()?;
        let over = over.intersect(&laid.text);
        // A line is in the region when the middle of its row is.
        let mut lines = laid.rows.iter().filter(|row| {
            let middle = row.origin.y + laid.line_height / 2.;
            over.top() <= middle && middle < over.bottom()
        });
        let first = lines.next()?.line;
        let last = lines.next_back().map_or(first, |row| row.line);
        let text = (first..=last)
            .map(|line| {
                self.file
                    .buffer
                    .line(line)
                    .trim_end_matches(['\n', '\r'])
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let lines = Lines {
            first: first + 1,
            last: last + 1,
        };
        let name = crate::chat::hand::of_file(&self.file.path, Some(lines), String::new()).label;
        let span = if first == last {
            format!("line {}", lines.first)
        } else {
            format!("lines {}-{}", lines.first, lines.last)
        };
        Some(Described {
            name,
            text: format!(
                "{span} of {}, in a tab of code:\n\n{}",
                self.file.path.display(),
                region::fenced(&text)
            ),
        })
    }

    /// The line of the cursor Vim's keys move, with the file it is
    /// from, as it goes to the agent when handed to the chat.
    fn cursor_attachment(&self) -> Option<Attachment> {
        let line = self.look.cursor?.line;
        let lines = Lines {
            first: line + 1,
            last: line + 1,
        };
        let text = self.file.buffer.line(line).into_owned();
        Some(crate::chat::hand::of_file(
            &self.file.path,
            Some(lines),
            text,
        ))
    }

    /// How many lines half the text in sight is, as last laid out.
    fn half_page(&self) -> usize {
        self.look.laid.as_ref().map_or(1, |laid| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a count of lines in sight, not below 0"
            )]
            let half =
                (f32::from(laid.text.size.height) / f32::from(laid.line_height) / 2.) as usize;
            half.max(1)
        })
    }

    /// Moves the cursor as Vim's keys ask, and scrolls with it.
    fn move_by_keys(&mut self, motion: &Move, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.file.buffer.lines().saturating_sub(1);
        let at = self
            .look
            .cursor
            .map_or_else(|| self.in_sight().start, |cursor| cursor.line);
        let line = motion.moved(at, self.half_page(), last);
        if matches!(motion.motion, Motion::HalfDown | Motion::HalfUp)
            && let Some(laid) = &self.look.laid
        {
            #[expect(
                clippy::cast_precision_loss,
                reason = "line counts are far below what f32 keeps exactly"
            )]
            let by = laid.line_height * (line as f32 - at as f32);
            self.look.scroll.y = (self.look.scroll.y + by).max(px(0.));
        }
        self.menu = None;
        self.look
            .put_cursor(self.look.cursor_on(line), &self.file.buffer);
        self.look.follow = Follow::Cursor;
        self.rest_on_cursor(cx);
        cx.notify();
    }

    /// Moves the cursor across the text as Vim's keys ask, by characters
    /// and words, and scrolls with it.
    fn move_across(&mut self, across: &MoveAcross, _: &mut Window, cx: &mut Context<Self>) {
        let buffer = &self.file.buffer;
        let lines: Vec<String> = (0..buffer.lines())
            .map(|line| buffer.line(line).into_owned())
            .collect();
        let at = self
            .look
            .cursor_on(self.cursor().unwrap_or_else(|| self.in_sight().start));
        let times = across
            .count
            .map_or(1, |count| usize::try_from(count).unwrap_or(usize::MAX));
        self.menu = None;
        self.look.put_cursor(
            cursor::moved(across.across, at, times, &lines),
            &self.file.buffer,
        );
        self.look.follow = Follow::Cursor;
        self.rest_on_cursor(cx);
        cx.notify();
    }

    /// The cursor came to rest: on a word, the server is asked of it, as
    /// of a word the mouse rests on, and else what it said goes.
    fn rest_on_cursor(&mut self, cx: &mut Context<Self>) {
        let word = self.cursor_at().and_then(|place| {
            let line = self.file.buffer.line(place.line);
            let word = word_around(&line, place.column);
            line[word.clone()]
                .starts_with(is_word)
                .then_some((place.line, word))
        });
        if word != self.pointed {
            self.hover = None;
        }
        self.rest_on(word, cx);
    }

    /// Starts selecting whole lines from the cursor's, or stops (`V`).
    fn visual_lines(&mut self, _: &crate::vim::Visual, _: &mut Window, cx: &mut Context<Self>) {
        self.visual(Visual::Lines, cx);
    }

    /// Starts selecting characters from the cursor's, or stops (`v`).
    fn visual_characters(&mut self, _: &VisualCharacters, _: &mut Window, cx: &mut Context<Self>) {
        self.visual(Visual::Characters, cx);
    }

    /// Starts selecting as `visual` says from the cursor, or selects so
    /// from where the other started, or stops if it selects so already.
    fn visual(&mut self, visual: Visual, cx: &mut Context<Self>) {
        let cursor = self
            .look
            .cursor_on(self.cursor().unwrap_or_else(|| self.in_sight().start));
        match self.look.visual {
            Some((_, was)) if was == visual => {
                self.look.end_visual();
            }
            Some((from, _)) => self.look.visual = Some((from, visual)),
            None => {
                let from = cursor.place(&self.file.buffer.line(cursor.line));
                self.look.visual = Some((from, visual));
            }
        }
        self.look.put_cursor(cursor, &self.file.buffer);
        cx.notify();
    }

    /// Scrolls the cursor's line to the top, the middle or the bottom
    /// (`zt`, `zz`, `zb`).
    fn scroll_to(&mut self, to: &ScrollTo, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(line), Some(laid)) = (self.cursor(), &self.look.laid) else {
            return;
        };
        self.look.scroll.y = text::scrolled_for(to.sight, line, laid);
        self.look.follow = Follow::Scroll;
        self.hover = None;
        self.menu = None;
        cx.notify();
    }

    /// Finds the whole word under the cursor, or the next on its line,
    /// as the last search, from it (`*`, `#`).
    fn find_word(&mut self, asked: &FindWord, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place) = self.cursor_at() else {
            return;
        };
        let line = self.file.buffer.line(place.line);
        let Some(column) = line[place.column..]
            .char_indices()
            .find(|(_, c)| is_word(*c))
            .map(|(at, _)| place.column + at)
        else {
            return;
        };
        let word = word_around(&line, column);
        let find = Find {
            text: line[word.clone()].to_owned(),
            direction: asked.direction,
            word: true,
        };
        let search = Search {
            text: find.text.clone(),
            direction: find.direction,
            word: true,
        };
        crate::vim::searched(search, cx);
        self.look.end_visual();
        self.look.selection = Some((
            Place::new(place.line, word.start),
            Place::new(place.line, word.end),
        ));
        self.find_text(&find, window, cx);
    }

    /// Selects the next match of what Vim's `/` or `n` looks for, from
    /// the one selected or the cursor's line, with the cursor on its line,
    /// and marks the others.
    fn find_text(&mut self, find: &Find, _: &mut Window, cx: &mut Context<Self>) {
        let buffer = &self.file.buffer;
        let lines: Vec<String> = (0..buffer.lines())
            .map(|line| buffer.line(line).into_owned())
            .collect();
        let from = match self.look.selection {
            Some((anchor, head)) => {
                let start = anchor.min(head);
                (start.line, start.column)
            }
            None => (self.cursor().unwrap_or_else(|| self.in_sight().start), 0),
        };
        self.look.found = Some(find.clone());
        match search::next(&lines, from, &find.text, find.word, find.direction) {
            Some((line, range)) => {
                self.look.end_visual();
                self.look.selection =
                    Some((Place::new(line, range.start), Place::new(line, range.end)));
                let start = Place::new(line, range.start);
                self.look.put_cursor(
                    Cursor::at(start, &self.file.buffer.line(line)),
                    &self.file.buffer,
                );
                self.look.follow = Follow::Cursor;
                self.notice = None;
            }
            None => self.notice = Some(format!("{} not found", find.text)),
        }
        cx.notify();
    }

    /// The name under the cursor, or with the cursor on no name, the
    /// first name of its line: its line and the bytes of it.
    fn name_at_cursor(&self) -> Option<(usize, Range<usize>)> {
        let place = self.cursor_at()?;
        let text = self.file.buffer.line(place.line);
        let highlights = self.file.highlights(place.line);
        let word = word_around(&text, place.column);
        let column = if is_name(&text, &word, &highlights) {
            word.start
        } else {
            first_name(&text, &highlights)?
        };
        Some((place.line, word_around(&text, column)))
    }

    /// Goes to where the name under the cursor is defined, or with the
    /// cursor on no name, the first name of its line.
    fn definition(&mut self, _: &Definition, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((line, name)) = self.name_at_cursor() {
            self.look.end_visual();
            self.go_to_definition(Place::new(line, name.start), cx);
        }
    }

    /// Lists where the name under the cursor is used, or with the cursor
    /// on no name, the first name of its line.
    fn references(&mut self, _: &References, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((line, name)) = self.name_at_cursor() {
            self.look.end_visual();
            self.find_references(Place::new(line, name.start), cx);
        }
    }

    /// Shows at once what the server says of the name under the cursor,
    /// or with the cursor on no name, of the first name of its line.
    fn look_up_name(&mut self, _: &LookUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((line, name)) = self.name_at_cursor() {
            self.look_up(line, name, cx);
        }
    }
}

/// Where the first name of `line` starts, of the words the syntax says,
/// as `highlights`, are no keyword, comment, string or literal.
fn first_name(line: &str, highlights: &[(Range<usize>, Highlight)]) -> Option<usize> {
    let mut column = 0;
    while column < line.len() {
        let word = word_around(line, column);
        if is_name(line, &word, highlights) {
            return Some(word.start);
        }
        column = word.end.max(column + 1);
    }
    None
}

/// Whether `word`, bytes of `line`, is a name: a word the syntax says,
/// as `highlights`, is no keyword, comment, string or literal.
fn is_name(line: &str, word: &Range<usize>, highlights: &[(Range<usize>, Highlight)]) -> bool {
    let named = line[word.clone()].starts_with(|c: char| c.is_alphabetic() || c == '_');
    named
        && !highlights.iter().any(|(range, highlight)| {
            range.contains(&word.start)
                && matches!(
                    highlight,
                    Highlight::Keyword
                        | Highlight::Comment
                        | Highlight::String
                        | Highlight::Literal
                )
        })
}

impl Follows for Code {
    fn followed(&self) -> (PathBuf, Option<Stamp>) {
        (self.file.path.clone(), self.file.stamp)
    }

    fn looked(&mut self, looked: std::io::Result<(Stamp, String)>, cx: &mut Context<Self>) {
        match looked {
            Ok((stamp, text)) => {
                self.file.stamp = Some(stamp);
                self.file.unread = None;
                self.set_text(&text, cx);
            }
            Err(error) => self.file.unread = Some(error.to_string()),
        }
        cx.notify();
    }
}

impl Shows for Code {
    /// The file's name.
    fn title(&self) -> String {
        self.file.title()
    }

    fn kept(&self) -> KeptTab {
        KeptTab::File {
            path: self.file.path.clone(),
            lines: self.look.marked,
        }
    }
}

impl ShowsFile for Code {
    /// The project's server for the file's language, when Cox has them,
    /// is asked what the code means.
    fn open(
        cwd: &Path,
        path: PathBuf,
        text: &str,
        lines: Option<Lines>,
        cx: &mut Context<Self>,
    ) -> Self {
        let server = cx
            .try_global::<Servers>()
            .cloned()
            .and_then(|servers| servers.of(cwd, &path, cx));
        let mut code = Self::new(path, text, lines, server, cx);
        code.project = Some(cwd.to_owned());
        code
    }

    fn path(&self) -> &Path {
        &self.file.path
    }

    fn show(&mut self, text: &str, lines: Option<Lines>, cx: &mut Context<Self>) {
        self.show(text, lines, cx);
    }

    fn content(this: Entity<Self>) -> tab::TabContent {
        tab::TabContent::Code(this)
    }

    fn of(content: &tab::TabContent) -> Option<&Entity<Self>> {
        match content {
            tab::TabContent::Code(code) => Some(code),
            tab::TabContent::Page { .. }
            | tab::TabContent::Document(_)
            | tab::TabContent::Files(_)
            | tab::TabContent::Changes(_)
            | tab::TabContent::Flow(_)
            | tab::TabContent::Terminal(_)
            | tab::TabContent::Android(_)
            | tab::TabContent::Devices(_)
            | tab::TabContent::MacWindow(_)
            | tab::TabContent::Windows(_) => None,
        }
    }
}

impl Focusable for Code {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Selected for Code {
    /// The text selected: in what the mouse rests on, if any of it is,
    /// or else in the code.
    fn chosen_text(&self) -> String {
        self.hover
            .as_ref()
            .map(hover::Hovered::selected_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| self.selected_text())
    }

    /// The text selected in the code, or with nothing selected the line
    /// of the cursor Vim's keys move.
    fn attachment(&self) -> Option<Attachment> {
        match self.look.selection {
            None => self.cursor_attachment(),
            Some(_) => Some(self.selected_attachment()),
        }
    }

    fn end_visual(&mut self) -> bool {
        self.look.end_visual()
    }
}

impl Render for Code {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .map(|code| selected::on_actions(code, cx))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::move_by_keys))
            .on_action(cx.listener(Self::move_across))
            .on_action(cx.listener(Self::visual_lines))
            .on_action(cx.listener(Self::visual_characters))
            .on_action(cx.listener(|code, _: &crate::vim::LeaveVisual, _, cx| {
                if code.look.end_visual() {
                    cx.notify();
                }
            }))
            .on_action(cx.listener(Self::definition))
            .on_action(cx.listener(Self::references))
            .on_action(cx.listener(Self::look_up_name))
            .on_action(cx.listener(Self::find_word))
            .on_action(cx.listener(Self::scroll_to))
            .on_action(cx.listener(Self::find_text))
            .child(self.render_bar(cx))
            .child(
                div()
                    .id("code")
                    .debug_selector(|| "code".to_owned())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::right_mouse_down))
                    .on_mouse_down(
                        MouseButton::Navigate(NavigationDirection::Back),
                        cx.listener(|code, _, _, cx| code.go_back(cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Navigate(NavigationDirection::Forward),
                        cx.listener(|code, _, _, cx| code.go_forward(cx)),
                    )
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|code, _, _, _| code.look.selecting = false),
                    )
                    .on_hover(cx.listener(|code, hovered: &bool, _, cx| {
                        if !*hovered {
                            code.rest_on(None, cx);
                        }
                    }))
                    .on_scroll_wheel(cx.listener(Self::scroll))
                    .child(Text { code: cx.entity() })
                    .child(region::mark({
                        let code = cx.weak_entity();
                        move |_, over, cx| code.upgrade()?.read(cx).described(over)
                    }))
                    .children(self.render_hover(cx))
                    .children(self.render_menu(cx)),
            )
            .children(self.render_locations(cx))
    }
}

#[cfg(test)]
mod tests {
    use cox_editor::language::Highlight;

    use super::first_name;

    #[test]
    fn the_first_name_of_a_line_is_no_keyword_or_literal() {
        let keyword = [(0..3, Highlight::Keyword)];
        assert_eq!(first_name("let line_3 = 3;", &keyword), Some(4));
        assert_eq!(first_name("    main();", &[]), Some(4));
        assert_eq!(first_name("42 + x", &[]), Some(5));
        assert_eq!(first_name("// note", &[(0..7, Highlight::Comment)]), None);
        assert_eq!(first_name("", &[]), None);
    }
}
