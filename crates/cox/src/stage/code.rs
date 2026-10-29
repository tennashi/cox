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
//! [`places`]). Going to a place in another file, the tab shows that file,
//! as a page goes to another: back and forward go between the places the
//! tab went to.

mod hover;
mod places;

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};

use cox_core::tabs::Lines;
use cox_editor::buffer::{Buffer, Point as Place};
use cox_editor::language::{Highlight, Language};
use cox_editor::lsp::{Client, Document, Encoding, Position};
use cox_editor::syntax::Syntax;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, Element, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection, Pixels, Point,
    ScrollWheelEvent, ShapedLine, SharedString, Style, Subscription, Task, TextRun, Window,
    actions, anchored, deferred, div, fill, font, point, prelude::*, px, relative, rgb, size,
};

use crate::stage::follow::{self, Follows, Stamp};
use crate::stage::servers::LanguageServer;
use crate::ui::paths::shortened;
use crate::ui::theme::{BACKGROUND, DIM, FAILED, FONT, FOREGROUND, LINK, SELECTION};
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

const FONT_SIZE: f32 = 13.;
const LINE_HEIGHT: f32 = 1.4;

/// Room between the text and the tab's edges.
const PADDING: f32 = 8.;

/// How many columns apart the stops of a tab character are.
const TAB_STOPS: usize = 4;

/// The most of a line the tab draws, in bytes, so that a line as long as a
/// minified file's does not hold up every frame.
const LINE_LENGTH: usize = 4096;

/// How many lines a turn of the wheel scrolls.
const WHEEL_LINES: f32 = 3.;

/// Where the lines the tab is shown at go, from the top, as a part of its
/// height.
const REVEAL_AT: f32 = 1. / 3.;

const NUMBER: u32 = 0x6e_6e6e;
const NUMBER_MARKED: u32 = 0xd0_d0d0;
const MARKED: u32 = 0x33_3322;
/// The name the server says something of.
const HOVERED: u32 = 0x2c_3a4a;
const MENU: u32 = 0x25_2526;
const MENU_BORDER: u32 = 0x45_4545;
const MENU_HOVER: u32 = 0x04_395e;

/// How code that is nothing in particular is drawn.
const PLAIN: (u32, FontWeight, FontStyle) = (FOREGROUND, FontWeight::NORMAL, FontStyle::Normal);

/// How code is drawn, by what its syntax says it is: its color, and
/// whether it is bold or italic.
fn style(highlight: Highlight) -> (u32, FontWeight, FontStyle) {
    let color = match highlight {
        Highlight::Keyword
        | Highlight::VariableBuiltin
        | Highlight::Tag
        | Highlight::Heading
        | Highlight::PunctuationSpecial => 0x56_9cd6,
        Highlight::Function => 0xdc_dcaa,
        Highlight::Type | Highlight::TypeBuiltin | Highlight::Constructor => 0x4e_c9b0,
        Highlight::String | Highlight::Literal => 0xce_9178,
        Highlight::StringSpecial | Highlight::Escape => 0xd7_ba7d,
        Highlight::Number | Highlight::ConstantBuiltin => 0xb5_cea8,
        Highlight::Comment => 0x6a_9955,
        Highlight::Constant | Highlight::Uri => 0x4f_c1ff,
        Highlight::Property | Highlight::Variable | Highlight::Link => 0x9c_dcfe,
        Highlight::Attribute | Highlight::Label => 0xc5_86c0,
        Highlight::Operator | Highlight::Punctuation | Highlight::Emphasis | Highlight::Strong => {
            FOREGROUND
        }
    };
    let weight = match highlight {
        Highlight::Heading | Highlight::Strong => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    };
    let style = match highlight {
        Highlight::Emphasis => FontStyle::Italic,
        _ => FontStyle::Normal,
    };
    (color, weight, style)
}

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
    /// The file, as an absolute path.
    path: PathBuf,
    buffer: Buffer,
    /// How many characters the longest line has, which says how far the
    /// tab scrolls across.
    longest: usize,
    /// The language of the file, if the editor knows it.
    language: Option<&'static Language>,
    /// The syntax of the text, once parsed: until it is parsed again, as
    /// the text changes, it is only edited as the text changed.
    syntax: Option<Syntax>,
    /// What the stretches of the lines drawn so far are, by line, as the
    /// syntax says, until the text or the syntax changes: a line coming
    /// into sight is the only one worked out.
    highlighted: HashMap<usize, Spans>,
    /// The file's, as the tab last read it, or `None` until the tab looks
    /// at the file.
    stamp: Option<Stamp>,
    /// Why the file could not be read the last time the tab looked, if it
    /// could not.
    unread: Option<String>,
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
    /// The project's server that knows the language of the file, if the
    /// editor knows one.
    server: Option<Entity<LanguageServer>>,
    /// The file, open on the server once it is ready.
    document: Option<Document>,
    /// Where the tab was before it went to a place, the latest last.
    back: Vec<Visit>,
    /// Where the tab went back from, the latest last.
    forward: Vec<Visit>,
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
    places: Option<places::Places>,
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

/// Where the tab was: the file, what it marked and selected there, and
/// how far it was scrolled.
#[derive(Clone, Debug, PartialEq)]
struct Visit {
    path: PathBuf,
    marked: Option<Lines>,
    selection: Option<(Place, Place)>,
    scroll: Point<Pixels>,
}

/// The stretches of a line, each as bytes of it with what it is, in order.
type Spans = Vec<(Range<usize>, Highlight)>;

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
        let buffer = Buffer::new(text);
        // Once the server is ready, it gets the file.
        let server_changes = server.as_ref().map(|server| {
            cx.observe(server, |code, _, cx| {
                code.open_on_server(cx);
                cx.notify();
            })
        });
        let mut code = Self {
            longest: buffer.longest_line(),
            buffer,
            language: Language::of_path(&path),
            syntax: None,
            highlighted: HashMap::new(),
            path,
            stamp: None,
            unread: None,
            marked: lines,
            reveal: lines.map(|lines| lines.first - 1),
            scroll: Point::default(),
            selection: None,
            selecting: false,
            laid: None,
            server,
            document: None,
            back: Vec::new(),
            forward: Vec::new(),
            pointed: None,
            linking: false,
            hover: None,
            menu: None,
            places: None,
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
        &self.path
    }

    /// The name the tab goes by: the file's.
    #[must_use]
    pub fn title(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }

    /// The text shown.
    #[must_use]
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// The lines the tab was last shown at, if any.
    #[must_use]
    pub fn marked(&self) -> Option<Lines> {
        self.marked
    }

    /// The lines in sight, from 0, as the last frame drew them.
    #[must_use]
    pub fn in_sight(&self) -> std::ops::Range<usize> {
        let rows = self.laid.as_ref().map_or(&[][..], |laid| &laid.rows[..]);
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
        let laid = self.laid.as_ref()?;
        let at = laid.stretch(line, &(column..column + 1))?;
        Some(at.center())
    }

    /// What the stretches of `line`, from 0, are, as bytes of it, as the
    /// syntax says once parsed.
    #[must_use]
    pub fn highlights(&self, line: usize) -> Vec<(Range<usize>, Highlight)> {
        self.syntax.as_ref().map_or_else(Vec::new, |syntax| {
            syntax.highlights(&self.buffer, line, LINE_LENGTH)
        })
    }

    /// The text selected.
    #[must_use]
    pub fn selected_text(&self) -> String {
        match self.selection {
            Some((anchor, head)) => self.buffer.slice(anchor.min(head)..anchor.max(head)),
            None => String::new(),
        }
    }

    /// Shows `text`, which the file says now, at `lines` if any, which the
    /// tab scrolls to.
    pub fn show(&mut self, text: &str, lines: Option<Lines>, cx: &mut Context<Self>) {
        // Back goes to where the human was reading.
        if lines.is_some() && lines != self.marked {
            self.back.push(self.visit());
            self.forward.clear();
        }
        // The tab looks at the file again, which may have changed since.
        self.stamp = None;
        self.unread = None;
        self.set_text(text, cx);
        self.marked = lines;
        self.reveal = lines.map(|lines| lines.first - 1);
        cx.notify();
    }

    fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(edit) = self.buffer.set_text(text) {
            self.longest = self.buffer.longest_line();
            // What was selected may be gone.
            self.selection = None;
            self.selecting = false;
            if let Some(syntax) = &mut self.syntax {
                syntax.edit(&edit);
            }
            self.highlighted.clear();
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
        let Some(language) = self.language.filter(|language| {
            language.server().map(|server| server.program) == Some(server.name())
        }) else {
            return;
        };
        self.document = client.open(&self.path, language.protocol_name(), &self.buffer.text());
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
        request: impl FnOnce(&Client, &Document, Position) -> F,
        cx: &App,
    ) -> Option<F> {
        let client = self.client(cx)?;
        let document = self.document.as_ref()?;
        let position = client.encoding().position(&self.buffer, place);
        Some(request(client, document, position))
    }

    /// Where the tab is now.
    fn visit(&self) -> Visit {
        Visit {
            path: self.path.clone(),
            marked: self.marked,
            selection: self.selection,
            scroll: self.scroll,
        }
    }

    /// Shows the file at `path`, an absolute path, from its start, unless
    /// the tab shows it already: answers whether it does now.
    fn open(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if path == self.path {
            return true;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.notice = Some(format!("cannot read {}: {error}", shortened(path)));
                return false;
            }
        };
        path.clone_into(&mut self.path);
        self.buffer = Buffer::new(&text);
        self.longest = self.buffer.longest_line();
        self.language = Language::of_path(path);
        self.syntax = None;
        self.highlighted.clear();
        self.stamp = None;
        self.unread = None;
        self.marked = None;
        self.reveal = None;
        self.scroll = Point::default();
        self.selection = None;
        self.selecting = false;
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
        let same_file = path == self.path;
        if !self.open(path, cx) {
            return;
        }
        let encoding = self.encoding(cx);
        let start = encoding.point(&self.buffer, range.start);
        let end = encoding.point(&self.buffer, range.end).max(start);
        self.selection = Some((start, end));
        self.marked = Some(Lines {
            first: start.line + 1,
            last: end.line + 1,
        });
        // A line in sight stays where it is.
        if !(same_file && in_sight.contains(&start.line)) {
            self.reveal = Some(start.line);
        }
        if self.visit() != here {
            self.back.push(here);
            self.forward.clear();
        }
        cx.notify();
    }

    /// Goes to where the tab was at `visit`: answers whether it could.
    fn revisit(&mut self, visit: &Visit, cx: &mut Context<Self>) -> bool {
        if !self.open(&visit.path, cx) {
            return false;
        }
        self.marked = visit.marked;
        self.selection = visit.selection;
        self.scroll = visit.scroll;
        self.reveal = None;
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
        let Some(visit) = self.back.pop() else {
            return;
        };
        let here = self.visit();
        if self.revisit(&visit, cx) {
            self.forward.push(here);
        }
    }

    /// Goes forward again to where the tab went back from.
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        let Some(visit) = self.forward.pop() else {
            return;
        };
        let here = self.visit();
        if self.revisit(&visit, cx) {
            self.back.push(here);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_none() && self.hover.take().is_none() {
            self.places = None;
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
        let Some(language) = self.language else {
            return;
        };
        let buffer = self.buffer.clone();
        let old = self.syntax.clone();
        // A parse of the text before, if not done yet, is left to end.
        self._parse = cx.spawn(async move |code, cx| {
            let syntax = cx
                .background_executor()
                .spawn(async move { Syntax::parse(language, &buffer, old.as_ref()) })
                .await;
            code.update(cx, |code, cx| {
                code.syntax = syntax;
                code.highlighted.clear();
                cx.notify();
            })
            .ok();
        });
    }

    /// What the stretches of `line` are, as the last frames worked out, or
    /// else worked out now and added to `highlighted`, for the tab to keep.
    fn highlights_at(&self, line: usize, highlighted: &mut Vec<(usize, Spans)>) -> Spans {
        match (self.highlighted.get(&line), &self.syntax) {
            (Some(highlights), _) => highlights.clone(),
            (None, Some(syntax)) => {
                let highlights = syntax.highlights(&self.buffer, line, LINE_LENGTH);
                highlighted.push((line, highlights.clone()));
                highlights
            }
            (None, None) => Vec::new(),
        }
    }

    /// The place in the text under `position`: on the nearest line in
    /// sight, at the nearest character.
    fn place_at(&self, position: Point<Pixels>) -> Option<Place> {
        let laid = self.laid.as_ref()?;
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
        let laid = self.laid.as_ref()?;
        let row = laid.rows.iter().find(|row| {
            row.origin.y <= position.y && position.y < row.origin.y + laid.line_height
        })?;
        if !laid.text.contains(&position) || position.x < row.origin.x {
            return None;
        }
        let index = row.shaped.index_for_x(position.x - row.origin.x)?;
        let line = self.buffer.line(row.line);
        let word = word_around(&line, row.shown.column_of(index));
        line[word.clone()]
            .starts_with(is_word)
            .then_some((row.line, word))
    }

    /// The text of the word at `place`.
    fn word_text(&self, place: Place) -> String {
        let line = self.buffer.line(place.line);
        line[word_around(&line, place.column.min(line.len()))].to_owned()
    }

    /// Where in the window the bottom left of `column` of `line` is, if the
    /// line is in sight.
    fn below(&self, line: usize, column: usize) -> Option<Point<Pixels>> {
        self.laid
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
        let (lines, longest) = (self.buffer.lines() as f32, self.longest as f32);
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
            self.selection = None;
            self.go_to_definition(Place::new(line, word.start), cx);
            return;
        }
        let Some(place) = self.place_at(event.position) else {
            return;
        };
        let line = self.buffer.line(place.line);
        self.selection = Some(match event.click_count {
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
        self.selecting = event.click_count == 1;
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
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some(place) = self.place_at(event.position)
            && let Some((_, head)) = &mut self.selection
            && *head != place
        {
            *head = place;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        // A click with no drag selects nothing.
        if self.selection.is_some_and(|(anchor, head)| anchor == head) {
            self.selection = None;
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
        let Some(laid) = &self.laid else {
            return;
        };
        let delta = event.delta.pixel_delta(laid.line_height * WHEEL_LINES);
        let most = self.most_scroll(laid);
        let scroll = point(
            (self.scroll.x - delta.x).clamp(px(0.), most.x),
            (self.scroll.y - delta.y).clamp(px(0.), most.y),
        );
        if scroll != self.scroll {
            self.scroll = scroll;
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

    fn render_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.marked.map(|lines| {
            if lines.first == lines.last {
                format!("line {}", lines.first)
            } else {
                format!("lines {}–{}", lines.first, lines.last)
            }
        });
        let server = self
            .server
            .as_ref()
            .and_then(|server| server.read(cx).said());
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex_shrink_0()
                .px_1()
                .rounded_sm()
                .map(|button| {
                    if enabled {
                        button
                            .cursor_pointer()
                            .text_color(rgb(FOREGROUND))
                            .hover(|style| style.bg(rgb(0x40_4040)))
                    } else {
                        button.text_color(rgb(0x55_5555))
                    }
                })
                .child(label)
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .text_color(rgb(DIM))
            .when(!self.back.is_empty() || !self.forward.is_empty(), |bar| {
                bar.child(
                    button("code-back", "‹", !self.back.is_empty())
                        .on_click(cx.listener(|code, _, _, cx| code.go_back(cx))),
                )
                .child(
                    button("code-forward", "›", !self.forward.is_empty())
                        .on_click(cx.listener(|code, _, _, cx| code.go_forward(cx))),
                )
            })
            .child(
                div()
                    .debug_selector(|| "code-path".to_owned())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(shortened(&self.path)),
            )
            .children(lines.map(|lines| div().flex_shrink_0().child(lines)))
            .children(self.notice.as_ref().map(|notice| {
                div()
                    .debug_selector(|| "code-notice".to_owned())
                    .flex_shrink_0()
                    .child(notice.clone())
            }))
            .children(server.map(|server| {
                div()
                    .debug_selector(|| "code-server".to_owned())
                    .min_w_0()
                    .max_w(px(240.))
                    .truncate()
                    .child(server)
            }))
            .children(self.unread.as_ref().map(|error| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(FAILED))
                    .child(format!("cannot read: {error}"))
            }))
    }

    /// The menu of the right button, if it is open.
    fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, place) = self.menu?;
        let click = if cfg!(target_os = "macos") {
            "⌘-click"
        } else {
            "Ctrl-click"
        };
        let item = |id: &'static str, label: &'static str, key: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .flex()
                .justify_between()
                .gap_4()
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(MENU_HOVER)))
                .child(label)
                .child(div().text_color(rgb(DIM)).child(key))
        };
        let menu = div()
            .occlude()
            .min_w(px(220.))
            .p_1()
            .bg(rgb(MENU))
            .border_1()
            .border_color(rgb(MENU_BORDER))
            .rounded_md()
            .shadow_md()
            .text_color(rgb(FOREGROUND))
            .on_mouse_down_out(cx.listener(|code, _, _, cx| {
                code.menu = None;
                cx.notify();
            }))
            .child(
                item("go-to-definition", "Go to definition", click).on_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.go_to_definition(place, cx);
                    },
                )),
            )
            .child(
                item("find-references", "Find references", "").on_click(cx.listener(
                    move |code, _, _, cx| {
                        code.menu = None;
                        code.find_references(place, cx);
                    },
                )),
            );
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

/// Whether `c` is of a word: a letter, a digit, or `_`.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The bytes of the word around `column` of `line`: letters, digits and
/// `_`, or else the one character there.
fn word_around(line: &str, column: usize) -> std::ops::Range<usize> {
    let Some(here) = line[column..].chars().next() else {
        return column..column;
    };
    if !is_word(here) {
        return column..column + here.len_utf8();
    }
    let start = line[..column]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(column, |(at, _)| at);
    let end = line[column..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(line.len(), |(at, _)| column + at);
    start..end
}

impl Follows for Code {
    fn followed(&self) -> (PathBuf, Option<Stamp>) {
        (self.path.clone(), self.stamp)
    }

    fn looked(&mut self, looked: std::io::Result<(Stamp, String)>, cx: &mut Context<Self>) {
        match looked {
            Ok((stamp, text)) => {
                self.stamp = Some(stamp);
                self.unread = None;
                self.set_text(&text, cx);
            }
            Err(error) => self.unread = Some(error.to_string()),
        }
        cx.notify();
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
                        cx.listener(|code, _, _, _| code.selecting = false),
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
            .children(self.render_places(cx))
    }
}

/// A line as it is drawn: with its tab characters as spaces to the next
/// stop, and cut at [`LINE_LENGTH`].
struct Shown {
    text: String,
    /// Where each character of the line starts, in the text drawn and in
    /// the line, in order.
    starts: Vec<(usize, usize)>,
    /// How long the line is, in bytes.
    len: usize,
}

impl Shown {
    fn of(line: &str) -> Self {
        let mut text = String::with_capacity(line.len().min(LINE_LENGTH));
        let mut starts = Vec::with_capacity(line.len().min(LINE_LENGTH));
        let mut columns = 0;
        for (at, c) in line.char_indices() {
            if text.len() >= LINE_LENGTH {
                break;
            }
            starts.push((text.len(), at));
            if c == '\t' {
                let spaces = TAB_STOPS - columns % TAB_STOPS;
                text.extend(std::iter::repeat_n(' ', spaces));
                columns += spaces;
            } else {
                text.push(c);
                columns += 1;
            }
        }
        Self {
            text,
            starts,
            len: line.len(),
        }
    }

    /// The byte of the line that `index` of the text drawn is at.
    fn column_of(&self, index: usize) -> usize {
        match self.starts.iter().rposition(|&(shown, _)| shown <= index) {
            Some(at) if self.starts[at].0 == index => self.starts[at].1,
            // Past the last character drawn, or within a tab's spaces,
            // which go to the next character.
            Some(at) => self.starts.get(at + 1).map_or(self.len, |&(_, line)| line),
            None => 0,
        }
    }

    /// The stretches of the text drawn, in order, each with how long it is
    /// and what it is, from `highlights`, what the stretches of the line
    /// are.
    fn runs(&self, highlights: &Spans) -> Vec<(usize, Option<Highlight>)> {
        let mut runs: Vec<(usize, Option<Highlight>)> = Vec::new();
        let mut highlights = highlights.iter().peekable();
        for (at, &(shown, column)) in self.starts.iter().enumerate() {
            let end = self
                .starts
                .get(at + 1)
                .map_or(self.text.len(), |&(end, _)| end);
            while highlights
                .next_if(|(range, _)| range.end <= column)
                .is_some()
            {}
            let highlight = highlights
                .peek()
                .filter(|(range, _)| range.start <= column)
                .map(|&&(_, highlight)| highlight);
            match runs.last_mut() {
                Some((len, last)) if *last == highlight => *len += end - shown,
                _ => runs.push((end - shown, highlight)),
            }
        }
        runs
    }

    /// The index of the text drawn that `column` of the line is at.
    fn index_of(&self, column: usize) -> usize {
        self.starts
            .iter()
            .find(|&&(_, line)| line >= column)
            .map_or(self.text.len(), |&(shown, _)| shown)
    }
}

/// Draws the lines in sight.
struct Text {
    code: Entity<Code>,
}

/// What a frame paints.
#[derive(Default)]
struct Painted {
    /// The marked lines', under everything.
    marked: Vec<Bounds<Pixels>>,
    /// Under the text, within it.
    selection: Vec<Bounds<Pixels>>,
    /// The name the server says something of, under the text.
    hovered: Option<Bounds<Pixels>>,
    /// The line under the name a click goes to the definition of, over
    /// the text.
    link: Option<Bounds<Pixels>>,
    /// The lines in sight, at their top left.
    text: Vec<(Point<Pixels>, ShapedLine)>,
    /// Their numbers, at their top left.
    numbers: Vec<(Point<Pixels>, ShapedLine)>,
    /// Where the text shows, which it is cut to.
    text_bounds: Bounds<Pixels>,
    line_height: Pixels,
    /// Where the mouse points with the pointing hand, over a name a click
    /// goes to the definition of.
    pointing: Option<Hitbox>,
}

impl Text {
    /// Works out what to paint in `bounds`, and keeps how it laid the text
    /// out.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "line counts are far below what f32 keeps exactly, and the \
                  lines in sight are counted from a scroll that is not negative"
    )]
    fn lay_out(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Painted {
        let font = font(FONT);
        let font_size = px(FONT_SIZE);
        let text_system = window.text_system();
        let cell_width = text_system
            .advance(text_system.resolve_font(&font), font_size, 'm')
            .map_or(px(8.), |advance| advance.width);
        let line_height = (font_size * LINE_HEIGHT).round();
        let run = |len, style| text_run(&font, len, style);
        let shape = |text: &str, runs: &[TextRun], window: &Window| {
            window.text_system().shape_line(
                SharedString::from(text.to_owned()),
                font_size,
                runs,
                None,
            )
        };

        let code = self.code.read(cx);
        let lines = code.buffer.lines();
        let digits = lines.to_string().len();
        let gutter = cell_width * digits as f32 + px(PADDING * 2.);
        let text_bounds = Bounds::new(
            bounds.origin + point(gutter, px(0.)),
            size((bounds.size.width - gutter).max(px(0.)), bounds.size.height),
        );
        let mut laid = Laid {
            text: text_bounds,
            line_height,
            cell_width,
            rows: Vec::new(),
        };
        // Once the tab knows its size, the lines it is shown at come into
        // sight, and the scroll stays within the text.
        let mut scroll = code.scroll;
        if let Some(line) = code.reveal {
            scroll.y = line_height * line as f32 - text_bounds.size.height * REVEAL_AT;
        }
        let most = code.most_scroll(&laid);
        let scroll = point(
            scroll.x.clamp(px(0.), most.x),
            scroll.y.clamp(px(0.), most.y),
        );
        let top = text_bounds.origin.y + px(PADDING) - scroll.y;
        let left = text_bounds.origin.x + px(PADDING) - scroll.x;
        let first = ((scroll.y - px(PADDING)).max(px(0.)) / line_height) as usize;
        let after = ((scroll.y + bounds.size.height) / line_height).ceil() as usize + 1;
        let marked = code
            .marked
            .map_or(0..0, |lines| lines.first - 1..lines.last);
        let selection = code
            .selection
            .map(|(anchor, head)| (anchor.min(head), anchor.max(head)));
        let mut highlighted = Vec::new();
        let mut painted = Painted {
            text_bounds,
            line_height,
            ..Painted::default()
        };
        for line in first..after.min(lines) {
            let y = top + line_height * line as f32;
            let shown = Shown::of(&code.buffer.line(line));
            let highlights = code.highlights_at(line, &mut highlighted);
            let shaped = shape(&shown.text, &text_runs(&font, &shown, &highlights), window);
            let is_marked = marked.contains(&line);
            if is_marked {
                painted.marked.push(Bounds::new(
                    point(bounds.origin.x, y),
                    size(bounds.size.width, line_height),
                ));
            }
            if let Some((from, width)) = selection
                .and_then(|selection| selected(line, &shown, &shaped, selection, cell_width))
            {
                painted
                    .selection
                    .push(Bounds::new(point(left + from, y), size(width, line_height)));
            }
            let number = (line + 1).to_string();
            let color = if is_marked { NUMBER_MARKED } else { NUMBER };
            let number = shape(
                &number,
                &[run(number.len(), (color, PLAIN.1, PLAIN.2))],
                window,
            );
            let x = bounds.origin.x + gutter - px(PADDING) - number.width;
            painted.numbers.push((point(x, y), number));
            painted.text.push((point(left, y), shaped.clone()));
            laid.rows.push(Row {
                line,
                origin: point(left, y),
                shaped,
                shown,
            });
        }
        (painted.hovered, painted.link) = code.names(&laid);
        self.code.update(cx, |code, _| {
            code.reveal = None;
            code.scroll = scroll;
            code.laid = Some(laid);
            code.highlighted.extend(highlighted);
        });
        painted
    }
}

/// The runs of `shown` in `font`, drawn as `highlights`, what the
/// stretches of its line are, say.
fn text_runs(font: &Font, shown: &Shown, highlights: &Spans) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = shown
        .runs(highlights)
        .into_iter()
        .map(|(len, highlight)| text_run(font, len, highlight.map_or(PLAIN, style)))
        .collect();
    // Even an empty line is shaped with a run.
    if runs.is_empty() {
        runs.push(text_run(font, 0, PLAIN));
    }
    runs
}

/// A run of `len` bytes of text in `font`, drawn as `style` says.
fn text_run(
    font: &Font,
    len: usize,
    (color, weight, style): (u32, FontWeight, FontStyle),
) -> TextRun {
    TextRun {
        len,
        font: Font {
            weight,
            style,
            ..font.clone()
        },
        color: Hsla::from(rgb(color)),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// Where on `line`, shown as `shown` and shaped as `shaped`, `selection`
/// starts, and how wide it is there, if it is on the line.
fn selected(
    line: usize,
    shown: &Shown,
    shaped: &ShapedLine,
    (start, end): (Place, Place),
    cell_width: Pixels,
) -> Option<(Pixels, Pixels)> {
    if !(start.line..=end.line).contains(&line) {
        return None;
    }
    let from = if line == start.line { start.column } else { 0 };
    let to = if line == end.line {
        end.column
    } else {
        shown.len
    };
    let x = |column| shaped.x_for_index(shown.index_of(column));
    // A selection that goes on to the next line covers the line break too.
    let line_break = if line < end.line { cell_width } else { px(0.) };
    let width = x(to) - x(from) + line_break;
    (width > px(0.)).then(|| (x(from), width))
}

impl IntoElement for Text {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Text {
    type RequestLayoutState = ();
    type PrepaintState = Painted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let mut painted = self.lay_out(bounds, window, cx);
        if painted.link.is_some() {
            painted.pointing = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
        }
        painted
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        painted: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = painted.line_height;
        if let Some(hitbox) = &painted.pointing {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        // The key that makes a click go to a definition underlines the name
        // under the mouse as it goes down, and stops as it comes up.
        let code = self.code.clone();
        window.on_modifiers_changed(move |event, _, cx| {
            code.update(cx, |code, cx| code.link(event.modifiers.secondary(), cx));
        });
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for at in &painted.marked {
                window.paint_quad(fill(*at, rgb(MARKED)));
            }
            for (origin, number) in &painted.numbers {
                number.paint(*origin, line_height, window, cx).ok();
            }
            let text = ContentMask {
                bounds: painted.text_bounds,
            };
            window.with_content_mask(Some(text), |window| {
                if let Some(at) = painted.hovered {
                    window.paint_quad(fill(at, rgb(HOVERED)));
                }
                for at in &painted.selection {
                    window.paint_quad(fill(*at, rgb(SELECTION)));
                }
                for (origin, line) in &painted.text {
                    line.paint(*origin, line_height, window, cx).ok();
                }
                if let Some(at) = painted.link {
                    window.paint_quad(fill(at, rgb(LINK)));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_tab_characters_as_spaces_to_the_next_stop() {
        let shown = Shown::of("\tab\tc");
        assert_eq!(shown.text, "    ab  c");
        // In the spaces of the first tab, and right on `a`.
        assert_eq!(shown.column_of(2), 1);
        assert_eq!(shown.column_of(4), 1);
        assert_eq!(shown.column_of(9), 5);
        assert_eq!(shown.index_of(3), 6);
        assert_eq!(shown.index_of(5), 9);
    }

    #[test]
    fn draws_each_stretch_of_a_line_as_its_syntax_says() {
        // The tab stands for four spaces, and `x` is within no stretch.
        let shown = Shown::of("\tlet x = 1;");
        let highlights = vec![(1..4, Highlight::Keyword), (9..10, Highlight::Number)];
        assert_eq!(
            shown.runs(&highlights),
            [
                (4, None),
                (3, Some(Highlight::Keyword)),
                (5, None),
                (1, Some(Highlight::Number)),
                (1, None),
            ]
        );
        assert_eq!(Shown::of("").runs(&highlights).first(), None);
    }

    #[test]
    fn cuts_a_long_line() {
        let shown = Shown::of(&"x".repeat(LINE_LENGTH * 2));
        assert_eq!(shown.text.len(), LINE_LENGTH);
        assert_eq!(shown.column_of(LINE_LENGTH), LINE_LENGTH * 2);
    }

    #[test]
    fn finds_the_word_around_a_place() {
        let line = "let 日本_x = a.b;";
        let word = |column| &line[word_around(line, column)];
        assert_eq!(word(0), "let");
        assert_eq!(word("let ".len() + 3), "日本_x");
        assert_eq!(word(line.find('=').unwrap()), "=");
        assert_eq!(word(line.len()), "");
    }
}
