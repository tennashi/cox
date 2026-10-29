//! One tab of the stage: where the browser's picture of a page is shown,
//! and where the human's mouse, keys and input-method text go to the page.
//! A toolbar above the picture holds the address and the history buttons.
//!
//! The picture arrives from CEF as BGRA pixels whenever the page changes;
//! the tab uploads it as a GPUI image and paints it over its bounds. The
//! dropdown of a `<select>` arrives as a picture of its own, painted over
//! the page where CEF says it goes.
//!
//! Over the page, the tab draws the agent's pointer: where the agent
//! points, clicks and types, as the relay saw it.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cox_core::tabs::Tab as Kept;
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Context, Corners, DispatchPhase, Div, Element,
    ElementId, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    KeyDownEvent, KeyUpEvent, Keystroke, LayoutId, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathBuilder, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent, Stateful, Style,
    Subscription, Task, UTF16Selection, Window, div, fill, point, prelude::*, px, quad, relative,
    rgb, size, white,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::browser::pointer::{AgentPointer, Look, Rect, Spot};
use crate::browser::relay::AgentInput;
use crate::browser::{
    Browser, Caret, EditCommand, Event, Frame, Modifiers, MouseButton, Navigation, TargetId,
};
use crate::stage::tab::Content;
use crate::ui::input::{LineInput, Submitted};

/// How many logical pixels one line of a wheel scroll moves the page.
const LINE_HEIGHT: f32 = 40.;

pub struct Page {
    browser: Browser,
    focus_handle: FocusHandle,
    /// The picture currently uploaded to the GPU.
    image: Option<Arc<RenderImage>>,
    /// The popup's picture, while one is open.
    popup: Option<Arc<RenderImage>>,
    /// Text the input method is composing, if any.
    composing: Option<String>,
    /// Where the browser is, as of the last `Navigated` event.
    navigation: Navigation,
    /// The line the human types an address into.
    address: Entity<LineInput>,
    /// The address last put in that line by the browser, to tell the
    /// human's own typing from it.
    address_shown: String,
    /// The agent's pointer, and how it looks as of the last render.
    pointer: AgentPointer,
    look: Option<Look>,
    /// Renders the tab again when the pointer's look next changes.
    _pointer_changes: Option<Task<()>>,
    _events: Task<()>,
    /// Checks now and then that the browser answered the last resize.
    _ticks: Task<()>,
    _submitted: Subscription,
}

/// The page asks for a tab of its own, showing this address: for a link
/// with `target="_blank"`, or `window.open`.
pub struct OpenTab(pub String);

impl EventEmitter<OpenTab> for Page {}

/// How often the tab checks on a resize the browser has not answered.
const TICK: Duration = Duration::from_millis(250);

impl Page {
    pub fn new(
        browser: Browser,
        mut events: UnboundedReceiver<Event>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            // Painted, created or closed: the tab looks different either way.
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |page, cx| {
                    match event {
                        Event::Navigated => page.navigated(cx),
                        Event::Popup(url) => cx.emit(OpenTab(url)),
                        _ => {}
                    }
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
            }
        });
        let ticking = browser.clone();
        let ticks = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |_, _| ticking.settle_resize()).is_err() {
                    break;
                }
            }
        });
        let navigation = browser.navigation();
        let address = cx.new(LineInput::new);
        address.update(cx, |address, cx| address.set_text(&navigation.url, cx));
        let submitted =
            cx.subscribe_in(&address, window, |page, address, Submitted, window, cx| {
                page.go(address, window, cx);
            });
        Self {
            browser,
            focus_handle: cx.focus_handle(),
            image: None,
            popup: None,
            composing: None,
            address_shown: navigation.url.clone(),
            navigation,
            address,
            pointer: AgentPointer::default(),
            look: None,
            _pointer_changes: None,
            _events: task,
            _ticks: ticks,
            _submitted: submitted,
        }
    }

    /// The address line.
    #[must_use]
    pub fn address(&self) -> &Entity<LineInput> {
        &self.address
    }

    /// The id `DevTools` knows the page by, once CEF has told it.
    #[must_use]
    pub fn target_id(&self) -> TargetId {
        self.browser.target_id()
    }

    /// Where the browser is, and what the page is called.
    #[must_use]
    pub fn navigation(&self) -> &Navigation {
        &self.navigation
    }

    /// The agent sent `input` to the page.
    pub fn see(&mut self, input: AgentInput, cx: &mut Context<Self>) {
        self.pointer.see(input, Instant::now());
        cx.notify();
    }

    /// The browser moved, or the page's title changed: show where, unless
    /// the human is in the middle of typing an address of their own.
    fn navigated(&mut self, cx: &mut Context<Self>) {
        self.navigation = self.browser.navigation();
        let url = self.navigation.url.clone();
        let typing = self.address.read(cx).text() != self.address_shown;
        if !typing && url != self.address_shown {
            self.address
                .update(cx, |address, cx| address.set_text(&url, cx));
            self.address_shown = url;
        }
    }

    /// The human pressed Enter in the address line: go there, and give the
    /// page the keyboard.
    fn go(&mut self, address: &Entity<LineInput>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = address_to_url(address.read(cx).text()) else {
            return;
        };
        self.open(&url, window, cx);
    }

    /// Opens `url`, and gives the page the keyboard.
    pub fn open(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.address
            .update(cx, |address, cx| address.set_text(url, cx));
        url.clone_into(&mut self.address_shown);
        self.browser.load(url);
        cx.focus_self(window);
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Navigation {
            loading,
            can_go_back,
            can_go_forward,
            ..
        } = self.navigation;
        let address = self.address.clone();
        div()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(rgb(0x40_4040))
            .child(Self::button("back", "◀", can_go_back, Browser::go_back, cx))
            .child(Self::button(
                "forward",
                "▶",
                can_go_forward,
                Browser::go_forward,
                cx,
            ))
            .child(if loading {
                Self::button("stop", "✕", true, Browser::stop, cx)
            } else {
                Self::button("reload", "↻", true, Browser::reload, cx)
            })
            .child(
                div()
                    .debug_selector(|| "address".to_owned())
                    .flex_1()
                    .px_2()
                    .rounded_sm()
                    .bg(rgb(0x2a_2a2a))
                    // A click brings the keyboard to the address line.
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |_, _, window, cx| {
                            window.focus(&address.focus_handle(cx));
                        }),
                    )
                    .child(self.address.clone()),
            )
    }

    /// A toolbar button that calls `action` on the browser when clicked,
    /// drawn dim and inert when `enabled` is false.
    fn button(
        name: &'static str,
        label: &'static str,
        enabled: bool,
        action: fn(&Browser),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let button = div()
            .id(name)
            .debug_selector(|| name.to_owned())
            .px_2()
            .rounded_sm()
            .child(label);
        if !enabled {
            return button.text_color(rgb(0x60_6060));
        }
        button
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x30_3030)))
            .on_click(cx.listener(move |stage, _, _, _| action(&stage.browser)))
    }

    /// Called from paint: tells the browser the stage's size, and picks up
    /// the newest picture.
    fn prepare_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Option<Arc<RenderImage>> {
        self.browser.resize(
            whole_pixels(bounds.size.width),
            whole_pixels(bounds.size.height),
            window.scale_factor(),
        );
        if let Some(image) = upload(self.browser.take_frame())
            && let Some(old) = self.image.replace(image)
        {
            window.drop_image(old).ok();
        }
        self.image.clone()
    }

    /// Called from paint: picks up the popup's newest picture, and where
    /// it goes over the stage, while one is open.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts are far below what f32 keeps exactly"
    )]
    fn prepare_popup(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Option<(Arc<RenderImage>, Bounds<Pixels>)> {
        let Some(rect) = self.browser.popup() else {
            if let Some(old) = self.popup.take() {
                window.drop_image(old).ok();
            }
            return None;
        };
        if let Some(image) = upload(self.browser.take_popup_frame())
            && let Some(old) = self.popup.replace(image)
        {
            window.drop_image(old).ok();
        }
        let popup = Bounds {
            origin: bounds.origin + point(px(rect.x as f32), px(rect.y as f32)),
            size: size(px(rect.width as f32), px(rect.height as f32)),
        };
        self.popup.clone().map(|image| (image, popup))
    }

    /// A key went down over the page. The browser's own shortcuts come
    /// first; the rest goes to the page.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if let Some(shortcut) = shortcut(keystroke) {
            match shortcut {
                Shortcut::Edit(command) => self.browser.edit(command),
                Shortcut::Reload => self.browser.reload(),
                Shortcut::Address => {
                    window.focus(&self.address.focus_handle(cx));
                }
            }
            cx.stop_propagation();
            return;
        }
        self.browser
            .key_down(&keystroke.key, modifiers(keystroke.modifiers));
    }

    fn key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, _cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        self.browser
            .key_up(&keystroke.key, modifiers(keystroke.modifiers));
    }
}

/// A picture from CEF as a GPUI image. GPUI keeps images as BGRA too, so
/// the bytes go in as they are.
fn upload(frame: Option<Frame>) -> Option<Arc<RenderImage>> {
    let frame = frame?;
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// A position in the window, as a position in the stage in whole logical
/// pixels.
fn in_stage(position: Point<Pixels>, bounds: Bounds<Pixels>) -> (i32, i32) {
    (
        whole_pixels(position.x - bounds.origin.x),
        whole_pixels(position.y - bounds.origin.y),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "pixel counts fit in i32; rounding is the point"
)]
fn whole_pixels(pixels: Pixels) -> i32 {
    f32::from(pixels).round() as i32
}

/// Where the picture goes: at its own size from the stage's top left
/// corner, so a picture painted for another size is cut off or leaves a
/// gap rather than being stretched.
#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below what f32 keeps exactly"
)]
fn image_bounds(image: &RenderImage, bounds: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let painted = image.size(0);
    Bounds {
        origin: bounds.origin,
        size: size(
            px(painted.width.0 as f32 / scale),
            px(painted.height.0 as f32 / scale),
        ),
    }
}

fn modifiers(modifiers: gpui::Modifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
        command: cfg!(target_os = "macos") && modifiers.platform,
    }
}

/// What the stage does with a keystroke itself, instead of handing it to
/// the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shortcut {
    Edit(EditCommand),
    Reload,
    /// Go to the address line.
    Address,
}

/// The browser's shortcuts: Command on macOS, Control elsewhere, with a
/// letter. Chromium with a window of its own gets these from the window's
/// menu; the stage has to supply them.
fn shortcut(keystroke: &Keystroke) -> Option<Shortcut> {
    let modifiers = keystroke.modifiers;
    if !modifiers.secondary() || modifiers.alt || modifiers.function {
        return None;
    }
    let shortcut = match (keystroke.key.as_str(), modifiers.shift) {
        ("z", false) => Shortcut::Edit(EditCommand::Undo),
        ("z", true) | ("y", false) => Shortcut::Edit(EditCommand::Redo),
        ("x", false) => Shortcut::Edit(EditCommand::Cut),
        ("c", false) => Shortcut::Edit(EditCommand::Copy),
        ("v", false) => Shortcut::Edit(EditCommand::Paste),
        ("a", false) => Shortcut::Edit(EditCommand::SelectAll),
        ("r", false) => Shortcut::Reload,
        ("l", false) => Shortcut::Address,
        _ => return None,
    };
    Some(shortcut)
}

fn button(button: gpui::MouseButton) -> Option<MouseButton> {
    match button {
        gpui::MouseButton::Left => Some(MouseButton::Left),
        gpui::MouseButton::Middle => Some(MouseButton::Middle),
        gpui::MouseButton::Right => Some(MouseButton::Right),
        gpui::MouseButton::Navigate(_) => None,
    }
}

fn clicks(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

impl Drop for Page {
    /// The page goes with its tab.
    fn drop(&mut self) {
        self.browser.close();
    }
}

impl Content for Page {
    /// The page's title, or its address until it has one.
    fn title(&self) -> String {
        if self.navigation.title.is_empty() {
            self.navigation.url.clone()
        } else {
            self.navigation.title.clone()
        }
    }

    fn kept(&self) -> Kept {
        Kept::Page {
            url: self.navigation.url.clone(),
            title: self.navigation.title.clone(),
        }
    }
}

impl Focusable for Page {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What an input method, or GPUI itself for plain typing, tells the stage.
/// The page keeps the text, so the stage only remembers what is being
/// composed.
impl EntityInputHandler for Page {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .composing
            .as_ref()
            .map_or(0, |text| text.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.composing
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.composing.take().is_some() {
            self.browser.finish_composition();
        }
        cx.notify();
    }

    /// Final text: typed characters, or a composition the input method
    /// committed.
    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composing = None;
        self.browser.commit(text);
        cx.notify();
    }

    /// Text the input method is still composing.
    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.browser.compose(new_text, new_selected_range_utf16);
        self.composing = Some(new_text.to_owned());
        cx.notify();
    }

    /// Where the input method puts its candidate window: at the page's
    /// composition caret, or the stage's top left corner before the page
    /// says.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel counts are far below what f32 keeps exactly"
    )]
    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let caret = self.browser.caret().unwrap_or(Caret {
            x: 0,
            y: 0,
            width: 1,
            height: 20,
        });
        Some(Bounds {
            origin: element_bounds.origin + point(px(caret.x as f32), px(caret.y as f32)),
            size: size(px(caret.width as f32), px(caret.height as f32)),
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

/// What the human typed in the address line, as a URL to load: a scheme is
/// taken as given, anything else is a host on the web. Blank is nothing.
fn address_to_url(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    // `about:blank` and `http://...` have a scheme; `localhost:3000` has a
    // port.
    let has_scheme = typed.split_once(':').is_some_and(|(scheme, rest)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !rest.starts_with(|c: char| c.is_ascii_digit())
    });
    Some(if has_scheme {
        typed.to_owned()
    } else {
        format!("https://{typed}")
    })
}

impl Page {
    /// Works out how the agent's pointer looks now, and arranges to render
    /// again when that changes on its own: the stage's paint asks for the
    /// next frame while it moves, and a timer wakes it for what comes
    /// later.
    #[expect(
        clippy::used_underscore_binding,
        reason = "the timer is only held: dropping it cancels it"
    )]
    fn update_look(&mut self, cx: &mut Context<Self>) {
        self.look = self.pointer.look(Instant::now());
        self._pointer_changes = self
            .look
            .as_ref()
            .map(|look| look.next_change)
            .filter(|next| !next.is_zero())
            .map(|next| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(next).await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                })
            });
    }

    /// The label by the agent's pointer, which says whose it is, and that
    /// the agent is typing.
    fn render_label(look: &Look) -> Div {
        div()
            .absolute()
            .left(px(look.pointer.x + 14.))
            .top(px(look.pointer.y + 18.))
            .opacity(look.opacity)
            .px_1p5()
            .rounded_sm()
            .bg(agent_color())
            .text_color(white())
            .text_xs()
            .whitespace_nowrap()
            .child(if look.typing {
                "Agent · typing"
            } else {
                "Agent"
            })
    }
}

impl Render for Page {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_look(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("stage")
                    .debug_selector(|| "stage".to_owned())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus_handle)
                    .key_context("Stage")
                    .on_key_down(cx.listener(Self::key_down))
                    .on_key_up(cx.listener(Self::key_up))
                    .child(StageElement { stage: cx.entity() })
                    .children(self.look.as_ref().map(Self::render_label)),
            )
    }
}

/// The color of everything the stage draws for the agent.
fn agent_color() -> Hsla {
    rgb(0x4f_6bff).into()
}

/// The agent's pointer at `look`, over the stage at `bounds`: the ring of
/// each click, the outline of the field it types into, and the arrow.
fn paint_agent(look: &Look, bounds: Bounds<Pixels>, window: &mut Window) {
    let color = agent_color();
    let on_stage = |spot: Spot| bounds.origin + point(px(spot.x), px(spot.y));
    if let Some(Rect {
        x,
        y,
        width,
        height,
    }) = look.field
    {
        let field = Bounds {
            origin: on_stage(Spot { x, y }) - point(px(3.), px(3.)),
            size: size(px(width + 6.), px(height + 6.)),
        };
        window.paint_quad(quad(
            field,
            px(4.),
            color.opacity(0.08 * look.opacity),
            px(2.),
            color.opacity(look.opacity),
            BorderStyle::Solid,
        ));
    }
    for ripple in &look.ripples {
        let radius = px(ripple.radius);
        let ring = Bounds {
            origin: on_stage(ripple.at) - point(radius, radius),
            size: size(radius * 2., radius * 2.),
        };
        window.paint_quad(quad(
            ring,
            radius,
            color.opacity(0.2 * ripple.opacity),
            px(2.),
            color.opacity(ripple.opacity),
            BorderStyle::Solid,
        ));
    }

    // An arrow with its tip on the spot, the shape of the usual pointer.
    let tip = on_stage(look.pointer);
    let corners = [
        (0., 0.),
        (0., 17.),
        (4.5, 13.),
        (7.5, 20.),
        (10., 19.),
        (7., 12.),
        (12.5, 12.),
    ]
    .map(|(x, y)| tip + point(px(x), px(y)));
    let mut arrow = PathBuilder::fill();
    arrow.add_polygon(&corners, true);
    let mut edge = PathBuilder::stroke(px(1.5));
    edge.add_polygon(&corners, true);
    if let Ok(arrow) = arrow.build() {
        window.paint_path(arrow, color.opacity(look.opacity));
    }
    if let Ok(edge) = edge.build() {
        window.paint_path(edge, white().opacity(look.opacity));
    }
}

/// Paints the browser's picture and sends mouse events over it to the page.
struct StageElement {
    stage: Entity<Page>,
}

impl StageElement {
    /// Paints the page, the popup over it, and the agent's pointer over
    /// both.
    fn paint_page(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let (image, popup, look) = self.stage.update(cx, |stage, _| {
            (
                stage.prepare_frame(bounds, window),
                stage.prepare_popup(bounds, window),
                stage.look.clone(),
            )
        });
        window.paint_quad(fill(bounds, rgb(0x33_3333)));
        if let Some(image) = image {
            let at = image_bounds(&image, bounds, window.scale_factor());
            window
                .paint_image(at, Corners::default(), image, 0, false)
                .ok();
        }
        if let Some((image, popup)) = popup {
            window
                .paint_image(popup, Corners::default(), image, 0, false)
                .ok();
        }
        if let Some(look) = look {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                paint_agent(&look, bounds, window);
            });
            if look.next_change.is_zero() {
                window.request_animation_frame();
            }
        }
    }
}

impl IntoElement for StageElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for StageElement {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

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
        _cx: &mut App,
    ) -> Self::PrepaintState {
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, browser) = {
            let stage = self.stage.read(cx);
            (stage.focus_handle.clone(), stage.browser.clone())
        };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.stage.clone()),
            cx,
        );

        self.paint_page(bounds, window, cx);

        let hovered = hitbox.clone();
        let target = browser.clone();
        window.on_mouse_event(
            move |event: &MouseDownEvent, phase: DispatchPhase, window, _| {
                if !phase.bubble() || !hovered.is_hovered(window) {
                    return;
                }
                window.focus(&focus_handle);
                if let Some(button) = button(event.button) {
                    let (x, y) = in_stage(event.position, bounds);
                    target.mouse_down(
                        x,
                        y,
                        button,
                        clicks(event.click_count),
                        modifiers(event.modifiers),
                    );
                }
            },
        );

        let target = browser.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase: DispatchPhase, _, _| {
            if !phase.bubble() {
                return;
            }
            if let Some(button) = button(event.button) {
                let (x, y) = in_stage(event.position, bounds);
                target.mouse_up(
                    x,
                    y,
                    button,
                    clicks(event.click_count),
                    modifiers(event.modifiers),
                );
            }
        });

        let hovered = hitbox.clone();
        let target = browser.clone();
        window.on_mouse_event(
            move |event: &MouseMoveEvent, phase: DispatchPhase, window, _| {
                // Moves outside the stage matter only while dragging from it.
                if !phase.bubble()
                    || (!hovered.is_hovered(window) && event.pressed_button.is_none())
                {
                    return;
                }
                let (x, y) = in_stage(event.position, bounds);
                let pressed = event.pressed_button.and_then(button);
                target.mouse_move(x, y, pressed, modifiers(event.modifiers));
            },
        );

        let hovered = hitbox.clone();
        window.on_mouse_event(
            move |event: &ScrollWheelEvent, phase: DispatchPhase, window, _| {
                if !phase.bubble() || !hovered.should_handle_scroll(window) {
                    return;
                }
                let delta = match event.delta {
                    ScrollDelta::Pixels(delta) => delta,
                    ScrollDelta::Lines(lines) => {
                        point(px(lines.x * LINE_HEIGHT), px(lines.y * LINE_HEIGHT))
                    }
                };
                let (x, y) = in_stage(event.position, bounds);
                browser.wheel(
                    x,
                    y,
                    whole_pixels(delta.x),
                    whole_pixels(delta.y),
                    modifiers(event.modifiers),
                );
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{EditCommand, Shortcut, address_to_url, shortcut};
    use gpui::Keystroke;

    fn parse(keystroke: &str) -> Keystroke {
        Keystroke::parse(keystroke).expect("a keystroke GPUI reads")
    }

    #[test]
    fn the_secondary_key_with_a_letter_is_a_shortcut() {
        let secondary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-c"))),
            Some(Shortcut::Edit(EditCommand::Copy))
        );
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-shift-z"))),
            Some(Shortcut::Edit(EditCommand::Redo))
        );
        assert_eq!(
            shortcut(&parse(&format!("{secondary}-l"))),
            Some(Shortcut::Address)
        );
    }

    #[test]
    fn other_keys_go_to_the_page() {
        assert_eq!(shortcut(&parse("c")), None);
        assert_eq!(shortcut(&parse("shift-c")), None);
        assert_eq!(shortcut(&parse("alt-c")), None);
        let secondary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(shortcut(&parse(&format!("{secondary}-k"))), None);
    }

    #[test]
    fn a_scheme_is_taken_as_given() {
        assert_eq!(
            address_to_url("http://localhost:3000/"),
            Some("http://localhost:3000/".to_owned())
        );
        assert_eq!(
            address_to_url("about:blank"),
            Some("about:blank".to_owned())
        );
    }

    #[test]
    fn a_host_is_on_the_web() {
        assert_eq!(
            address_to_url(" example.com/path "),
            Some("https://example.com/path".to_owned())
        );
        assert_eq!(
            address_to_url("localhost:3000"),
            Some("https://localhost:3000".to_owned())
        );
    }

    #[test]
    fn blank_is_nothing() {
        assert_eq!(address_to_url("  "), None);
    }
}
