//! One tab of the stage: where the browser's picture of a page is shown,
//! and where the human's mouse, keys and input-method text go to the page.
//! A toolbar above the picture holds the address and the history buttons.
//!
//! - [`picture`] paints the browser's picture, and sends the mouse over
//!   it to the page.
//! - [`keys`] sends the keys to the page, but for the browser's own
//!   shortcuts.
//!
//! Over the picture, the tab draws the agent's pointer: where the agent
//! points, clicks and types, as the relay saw it (see
//! [`crate::stage::pointer`]).
//!
//! The right button over text selected in the page opens a menu of what
//! to do with it, "Translate" and "Copy", as the browser shows none.

mod keys;
mod picture;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cox_core::entry::Attachment;
use cox_core::tabs::KeptTab;
use gpui::{
    App, Bounds, ClipboardItem, Context, Div, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, IntoElement, Pixels, Point, RenderImage, Stateful, Subscription, Task,
    UTF16Selection, Window, anchored, deferred, div, point, prelude::*, px, rgb, size,
};
use tokio::sync::mpsc::UnboundedReceiver;

use self::picture::Picture;
use crate::chat::region::{self, Described};
use crate::stage::pointer;
use crate::stage::tab::Shows;
use crate::ui::input::{LineInput, Submitted};
use crate::ui::theme::{ACCENT_GROUND, BORDER, BORDER_STRONG, BUTTON, DISABLED, RAISED, SELECTED};
use cox_browser::pointer::{AgentPointer, Look};
use cox_browser::relay::AgentInput;
use cox_browser::{Browser, Caret, Event, Navigation, TargetId};

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
    /// Where the page was drawn in the window last.
    origin: Point<Pixels>,
    /// The menu of the text selected in the page, where it opened, while
    /// it is open.
    menu: Option<(Point<Pixels>, String)>,
    _events: Task<()>,
    /// Checks now and then that the browser answered the last resize.
    _ticks: Task<()>,
    _submitted: Subscription,
    /// Tell the browser when the page gets the keyboard and loses it, so
    /// that the page shows its caret and focus rings just while it has it.
    _focus: [Subscription; 2],
}

/// What the page asks the stage to open.
pub enum Opens {
    /// A tab of its own, showing this address: for a link with
    /// `target="_blank"`, or `window.open`.
    Tab(String),
    /// A file the human clicked a link to, at this `file:` address with
    /// the lines after it if any, on a tab of its own, as a link to it
    /// from the conversation opens.
    File(String),
    /// The chat the window goes by this number for, which a plugin's page
    /// asks the window to show (see [`crate::stage::activity`]).
    Chat(u64),
}

impl EventEmitter<Opens> for Page {}

/// How often the tab checks on a resize the browser has not answered.
const TICK: Duration = Duration::from_millis(250);

impl Page {
    pub fn new(
        browser: Browser,
        mut events: UnboundedReceiver<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let focused = browser.clone();
        let blurred = browser.clone();
        let focus = [
            cx.on_focus(&focus_handle, window, move |_, _, _| focused.focus()),
            cx.on_blur(&focus_handle, window, move |_, _, _| blurred.blur()),
        ];
        let task = cx.spawn(async move |this, cx| {
            // Painted, created or closed: the tab looks different either way.
            while let Some(event) = events.recv().await {
                let shown = this.update(cx, |page, cx| {
                    match event {
                        Event::Navigated => page.navigated(cx),
                        Event::Popup(url) => cx.emit(Opens::Tab(url)),
                        Event::FileLink(url) => cx.emit(Opens::File(url)),
                        Event::ShowChat(key) => cx.emit(Opens::Chat(key)),
                        Event::Selected { x, y, text } => {
                            let at = page.origin + point(in_window(x), in_window(y));
                            page.menu = Some((at, text));
                        }
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
            focus_handle,
            image: None,
            popup: None,
            composing: None,
            address_shown: navigation.url.clone(),
            navigation,
            address,
            pointer: AgentPointer::default(),
            look: None,
            _pointer_changes: None,
            origin: Point::default(),
            menu: None,
            _events: task,
            _ticks: ticks,
            _submitted: submitted,
            _focus: focus,
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

    /// Whether the page is one the stage serves itself, at a `cox:`
    /// address (see [`crate::stage::plugins`]): drawn as a tab Cox draws,
    /// with no toolbar, since the page is the tab.
    fn is_plugin(&self) -> bool {
        self.navigation.url.starts_with("cox://")
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
            .border_color(rgb(BORDER))
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
                    .bg(rgb(SELECTED))
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
            return button.text_color(rgb(DISABLED));
        }
        button
            .cursor_pointer()
            .hover(|style| style.bg(rgb(BUTTON)))
            .on_click(cx.listener(move |stage, _, _, _| action(&stage.browser)))
    }
}

impl Drop for Page {
    /// The page goes with its tab.
    fn drop(&mut self) {
        self.browser.close();
    }
}

impl Shows for Page {
    /// The page's title, or its address until it has one.
    fn title(&self) -> String {
        if self.navigation.title.is_empty() {
            self.navigation.url.clone()
        } else {
            self.navigation.title.clone()
        }
    }

    fn kept(&self) -> KeptTab {
        KeptTab::Page {
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
pub(super) fn address_to_url(typed: &str) -> Option<String> {
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

/// A length of the page, in the browser's whole pixels, in the window's.
#[expect(
    clippy::cast_precision_loss,
    reason = "a page is not millions of pixels wide"
)]
fn in_window(length: i32) -> Pixels {
    px(length as f32)
}

impl Page {
    /// The menu of the text selected in the page, while it is open.
    fn render_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let (at, text) = self.menu.clone()?;
        let item = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(ACCENT_GROUND)))
                .child(label)
        };
        let copied = text.clone();
        let handed = Attachment {
            label: self.title(),
            uri: self.navigation.url.clone(),
            text: text.clone(),
            picture: None,
        };
        let menu = div()
            .occlude()
            .min_w(px(140.))
            .p_1()
            .bg(rgb(RAISED))
            .border_1()
            .border_color(rgb(BORDER_STRONG))
            .rounded_md()
            .shadow_md()
            .on_mouse_down_out(cx.listener(|page, _, _, cx| {
                page.menu = None;
                cx.notify();
            }))
            .child(
                item("translate-selection", "Translate").on_click(cx.listener(
                    move |page, _, window, cx| {
                        page.menu = None;
                        crate::ui::translation::show(&text, window, cx);
                        cx.notify();
                    },
                )),
            )
            .child(item("add-to-chat", "Add to chat").on_click(cx.listener(
                move |page, _, _, cx| {
                    page.menu = None;
                    crate::chat::hand::hand(handed.clone(), cx);
                    cx.notify();
                },
            )))
            .child(
                item("copy-selection", "Copy").on_click(cx.listener(move |page, _, _, cx| {
                    page.menu = None;
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()));
                    cx.notify();
                })),
            );
        Some(deferred(anchored().position(at).child(menu)).with_priority(2))
    }
}

impl Page {
    /// Where on the page `over`, a stretch of a region in the window's
    /// points, is, the page being drawn over `page`.
    fn described(&self, page: Bounds<Pixels>, over: Bounds<Pixels>) -> Described {
        let title = self.title();
        let url = &self.navigation.url;
        let at = over.origin - page.origin;
        Described {
            text: format!(
                "the page \"{title}\" at {url}, from {:.0},{:.0} to {:.0},{:.0} in CSS pixels \
                of its viewport, as `document.elementFromPoint` takes them",
                f32::from(at.x),
                f32::from(at.y),
                f32::from(at.x + over.size.width),
                f32::from(at.y + over.size.height),
            ),
            name: title,
        }
    }
}

impl Render for Page {
    #[expect(
        clippy::used_underscore_binding,
        reason = "the timer is only held: dropping it cancels it"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.look, self._pointer_changes) = pointer::look_now(&self.pointer, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(!self.is_plugin(), |page| {
                page.child(self.render_toolbar(cx))
            })
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
                    .child(Picture { page: cx.entity() })
                    .child(region::mark({
                        let page = cx.weak_entity();
                        move |whole, over, cx| Some(page.upgrade()?.read(cx).described(whole, over))
                    }))
                    .children(
                        self.look
                            .as_ref()
                            .map(|look| pointer::label(look, Point::default())),
                    ),
            )
            .children(self.render_menu(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::address_to_url;

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
