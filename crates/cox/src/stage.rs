//! The stage: the part of the window where the browser's picture is shown,
//! and where the human's mouse, keys and input-method text go to the page.
//! A toolbar above the picture holds the address and the history buttons.
//!
//! The picture arrives from CEF as BGRA pixels whenever the page changes;
//! the stage uploads it as a GPUI image and paints it over its bounds.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, Bounds, Context, Corners, DispatchPhase, Div, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderImage, ScrollDelta, ScrollWheelEvent,
    Stateful, Style, Subscription, Task, UTF16Selection, Window, div, fill, point, prelude::*, px,
    relative, rgb, size,
};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::browser::{Browser, Caret, EditCommand, Event, Modifiers, MouseButton, Navigation};
use crate::input::{LineInput, Submitted};

/// How many logical pixels one line of a wheel scroll moves the page.
const LINE_HEIGHT: f32 = 40.;

pub struct Stage {
    browser: Browser,
    focus_handle: FocusHandle,
    /// The picture currently uploaded to the GPU.
    image: Option<Arc<RenderImage>>,
    /// Text the input method is composing, if any.
    composing: Option<String>,
    /// Where the browser is, as of the last `Navigated` event.
    navigation: Navigation,
    /// The line the human types an address into.
    address: Entity<LineInput>,
    /// The address last put in that line by the browser, to tell the
    /// human's own typing from it.
    address_shown: String,
    _events: Task<()>,
    _submitted: Subscription,
}

impl Stage {
    pub fn new(
        browser: Browser,
        mut events: UnboundedReceiver<Event>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            // Painted, created or closed: the stage looks different either way.
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |stage, cx| {
                    if event == Event::Navigated {
                        stage.navigated(cx);
                    }
                    cx.notify();
                });
                if shown.is_err() {
                    break;
                }
            }
        });
        let navigation = browser.navigation();
        let address = cx.new(LineInput::new);
        address.update(cx, |address, cx| address.set_text(&navigation.url, cx));
        let submitted =
            cx.subscribe_in(&address, window, |stage, address, Submitted, window, cx| {
                stage.go(address, window, cx);
            });
        Self {
            browser,
            focus_handle: cx.focus_handle(),
            image: None,
            composing: None,
            address_shown: navigation.url.clone(),
            navigation,
            address,
            _events: task,
            _submitted: submitted,
        }
    }

    /// The address line.
    #[must_use]
    pub fn address(&self) -> &Entity<LineInput> {
        &self.address
    }

    /// The browser moved: show where, unless the human is in the middle of
    /// typing an address of their own.
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
        address.update(cx, |address, cx| address.set_text(&url, cx));
        self.address_shown.clone_from(&url);
        self.browser.load(&url);
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
        if let Some(frame) = self.browser.take_frame()
            && let Some(buffer) = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)
        {
            // GPUI keeps images as BGRA too, so the bytes go in as they are.
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
            if let Some(old) = self.image.replace(image) {
                window.drop_image(old).ok();
            }
        }
        self.image.clone()
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

impl Focusable for Stage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What an input method, or GPUI itself for plain typing, tells the stage.
/// The page keeps the text, so the stage only remembers what is being
/// composed.
impl EntityInputHandler for Stage {
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

impl Render for Stage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("stage")
                    .debug_selector(|| "stage".to_owned())
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus_handle)
                    .key_context("Stage")
                    .on_key_down(cx.listener(Self::key_down))
                    .on_key_up(cx.listener(Self::key_up))
                    .child(StageElement { stage: cx.entity() }),
            )
    }
}

/// Paints the browser's picture and sends mouse events over it to the page.
struct StageElement {
    stage: Entity<Stage>,
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

        let image = self
            .stage
            .update(cx, |stage, _| stage.prepare_frame(bounds, window));
        match image {
            Some(image) => {
                window
                    .paint_image(bounds, Corners::default(), image, 0, false)
                    .ok();
            }
            None => window.paint_quad(fill(bounds, rgb(0x33_3333))),
        }

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
