//! A tab of the stage that shows a file as it is written, such as code,
//! for the human to read beside the conversation. The agent shows it with
//! Cox's tool for that (see [`crate::agent::mcp`]), at the lines it talks about,
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

use cox_core::tabs::{KeptTab, Lines};
use cox_editor::buffer::{Point as Place, is_word, word_around};
use cox_editor::history::History;
use cox_editor::language::Highlight;
use cox_editor::line::Shown;
use cox_editor::lsp::{Client, Encoding, OpenFile, Position};
use cox_editor::syntax::Syntax;

use self::file::File;
use self::text::Text;
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    Pixels, Point, ScrollWheelEvent, ShapedLine, Subscription, Task, Window, actions, div, point,
    prelude::*, px, rgb, size,
};

use crate::stage::follow::{self, Follows, Stamp};
use crate::stage::servers::{LanguageServer, Servers};
use crate::stage::tab::{self, Shows, ShowsFile};
use crate::ui::paths::shortened;
use crate::ui::theme::BACKGROUND;
use crate::ui::{Copy, LinkClicked};

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

const KEY_CONTEXT: &str = "Code";

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
        // Once the server is ready, it gets the file.
        let server_changes = server.as_ref().map(|server| {
            cx.observe(server, |code, _, cx| {
                code.open_on_server(cx);
                cx.notify();
            })
        });
        let mut code = Self {
            file: File::new(path, text),
            look: Look {
                marked: lines,
                reveal: lines.map(|lines| lines.first - 1),
                ..Look::default()
            },
            server,
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
        let Some(language) = self.file.language.filter(|language| {
            language.server().map(|server| server.program) == Some(server.name())
        }) else {
            return;
        };
        self.document = client.open(
            &self.file.path,
            language.protocol_name(),
            &self.file.buffer.text(),
        );
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
        self.parse(cx);
        self.open_on_server(cx);
        true
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
        if self.menu.take().is_none() && self.hover.take().is_none() {
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
        let line = self.file.buffer.line(place.line);
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

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .hover
            .as_ref()
            .map(hover::Hovered::selected_text)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| self.selected_text());
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
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
        Self::new(path, text, lines, server, cx)
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
            | tab::TabContent::Tree(_)
            | tab::TabContent::Changes(_)
            | tab::TabContent::Flow(_)
            | tab::TabContent::Terminal(_) => None,
        }
    }
}

impl Focusable for Code {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
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
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::forward))
            .on_action(cx.listener(Self::dismiss))
            .child(self.render_bar(cx))
            .child(
                div()
                    .id("code")
                    .debug_selector(|| "code".to_owned())
                    .flex_1()
                    .min_h_0()
                    .bg(rgb(BACKGROUND))
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
                    .children(self.render_hover(cx))
                    .children(self.render_menu(cx)),
            )
            .children(self.render_locations(cx))
    }
}
