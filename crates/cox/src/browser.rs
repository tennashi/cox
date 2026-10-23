//! The browser behind the stage: CEF (Chromium) rendering a page with no
//! window of its own. CEF hands each painted picture to [`Shared`], and the
//! stage hands mouse, keys and input-method text back through [`Browser`].
//! CEF's remote debugging port stays on, so an agent can drive the same
//! page over CDP meanwhile. The port is one that is free when Cox starts,
//! so that two Cox can run side by side.
//!
//! CEF runs its own threads: the browser is created and painted on CEF's UI
//! thread, while GPUI owns the main thread. Everything the two sides share
//! sits behind one mutex. On macOS, CEF's UI thread has to be the main
//! thread, so GPUI gives it a turn every few milliseconds instead (see
//! [`Browser::pump`]).

use std::os::raw::c_int;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cef::args::Args;
use cef::sys::{
    cef_event_flags_t, cef_key_event_type_t, cef_mouse_button_type_t, cef_paint_element_type_t,
};
// The `wrap_*` macros name these without a path, so they have to be in scope.
use cef::rc::Rc as _;
use cef::{
    App, BrowserHost, BrowserProcessHandler, BrowserSettings, CefString, Client, CommandLine,
    CompositionUnderline, DisplayHandler, ImplApp, ImplBrowser, ImplBrowserHost,
    ImplBrowserProcessHandler, ImplClient, ImplCommandLine, ImplDisplayHandler, ImplFrame,
    ImplLifeSpanHandler, ImplLoadHandler, ImplRenderHandler, KeyEvent, KeyEventType,
    LifeSpanHandler, LoadHandler, MouseButtonType, MouseEvent, RenderHandler, Settings, WindowInfo,
    WrapApp, WrapBrowserProcessHandler, WrapClient, WrapDisplayHandler, WrapLifeSpanHandler,
    WrapLoadHandler, WrapRenderHandler, wrap_app, wrap_browser_process_handler, wrap_client,
    wrap_display_handler, wrap_life_span_handler, wrap_load_handler, wrap_render_handler,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// One picture CEF painted: the whole view, or the whole popup, in BGRA,
/// in device pixels.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Where the popup is over the view, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What happened on CEF's side, for the stage to redraw on.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// A new frame is waiting in [`Browser::take_frame`].
    Painted,
    /// The browser exists and takes input now.
    Created,
    /// The browser is gone.
    Closed,
    /// The page moved, or started or finished loading: see
    /// [`Browser::navigation`].
    Navigated,
}

/// Where the browser is in its history, as the toolbar shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Navigation {
    /// The address of the page being shown, or being loaded.
    pub url: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// A mouse button CEF knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// Modifier keys held during an event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each key is held or not, as CEF's flags have them"
)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    /// The Command key on macOS; nothing elsewhere.
    pub command: bool,
}

/// An editing command the page's focused element carries out, as a
/// keyboard shortcut would in a browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// Where the input method's caret is, in logical pixels within the stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The stage's size in logical pixels, and the window's scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    width: i32,
    height: i32,
    scale: f32,
}

impl Viewport {
    /// The size of the picture CEF paints for this viewport, in device
    /// pixels: it rounds up, as Chromium does.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "pixel counts are small and positive; rounding is the point"
    )]
    fn device_size(self) -> (u32, u32) {
        let device = |logical: i32| (logical.max(1) as f32 * self.scale).ceil() as u32;
        (device(self.width), device(self.height))
    }

    /// The viewport, at this scale, that CEF paints a picture of `device`
    /// pixels for: the one to report while waiting for it.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        reason = "pixel counts are small and positive; rounding is the point"
    )]
    fn painted_as(device: (u32, u32), scale: f32) -> Self {
        // Rounding up may overshoot by one when the scale is fractional;
        // one logical pixel less then lands exactly.
        let logical = |pixels: u32| {
            let guess = (pixels as f32 / scale).ceil() as i32;
            let fits = |logical: i32| (logical as f32 * scale).ceil() as u32 == pixels;
            (guess - 1..=guess)
                .find(|&logical| logical >= 1 && fits(logical))
                .unwrap_or(guess)
        };
        Self {
            width: logical(device.0),
            height: logical(device.1),
            scale,
        }
    }
}

/// How long CEF gets to paint at a new size on its own before it is asked
/// for a picture, and asked again.
const RESIZE_NUDGE: Duration = Duration::from_millis(200);

/// How long CEF gets to paint at a new size before it is taken to be
/// stuck at another.
const RESIZE_TIMEOUT: Duration = Duration::from_secs(1);

/// A size CEF was told of, while no picture of that size has come yet.
#[derive(Clone, Copy, Debug)]
struct Resizing {
    since: Instant,
    /// When CEF was last asked for a picture meanwhile.
    nudged: Option<Instant>,
}

/// What to ask CEF for in [`Browser::settle_resize`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// Lay the page out at the size `view_rect` reports now.
    Resize,
    /// Paint a picture of the page as it is.
    Picture,
}

/// State shared between CEF's threads and the stage.
struct Shared {
    /// The size the stage has now.
    wanted: Viewport,
    /// The size CEF was told of last, and answers [`RenderHandler::view_rect`]
    /// with. CEF holds its pictures back after a resize until one of the
    /// size it was told arrives, comparing against what `view_rect` says
    /// *now*: so the answer must not move until that picture came, or the
    /// renderer, which was told the old size, never paints a matching one.
    reported: Viewport,
    /// The resize in flight, if any.
    resizing: Option<Resizing>,
    /// The size of the last picture, in device pixels.
    painted: Option<(u32, u32)>,
    /// The newest picture, until the stage takes it.
    frame: Option<Frame>,
    /// The popup: the dropdown of a `<select>`, which Chromium draws apart
    /// from the page. Where it is while it is open, and its newest
    /// picture until the stage takes it.
    popup: Option<PopupRect>,
    popup_frame: Option<Frame>,
    host: Option<BrowserHost>,
    caret: Option<Caret>,
    navigation: Navigation,
}

type SharedRef = Arc<Mutex<Shared>>;

/// CEF's "no range" (`CefRange::InvalidRange()`): the C API ignores a null
/// range, so "replace nothing" has to be spelled out.
const NO_RANGE: cef::Range = cef::Range {
    from: u32::MAX,
    to: u32::MAX,
};

/// Which process of the app is loading CEF on macOS.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Process {
    /// `cox` itself, at `Cox.app/Contents/MacOS/Cox`.
    Main,
    /// A helper, at `Cox.app/Contents/Frameworks/<name>.app/Contents/MacOS/<name>`.
    Helper,
}

/// Loads CEF on macOS, where it is a framework in the app bundle that
/// nothing links at build time, so it has to be found and loaded before
/// any other `cef` call.
///
/// # Panics
///
/// When this executable is not inside the bundle `cargo xtask bundle`
/// makes, or the framework does not load.
#[cfg(target_os = "macos")]
pub fn load_framework(process: Process) {
    use std::os::unix::ffi::OsStrExt as _;

    let exe = std::env::current_exe().expect("the executable has a path");
    // From `Contents/MacOS/` of the app or the helper to `Contents/Frameworks/`
    // of the app.
    let frameworks = match process {
        Process::Main => "../Frameworks",
        Process::Helper => "../../..",
    };
    let framework = exe
        .parent()
        .expect("the executable is in a directory")
        .join(frameworks)
        .join(cef::sys::FRAMEWORK_PATH);
    let path = std::ffi::CString::new(framework.as_os_str().as_bytes())
        .expect("the framework path has no NUL");
    // SAFETY: `path` outlives the call, and CEF reads it as a C string.
    let loaded = unsafe { cef::load_library(Some(&*path.as_ptr())) } == 1;
    assert!(
        loaded,
        "CEF did not load from {}; run Cox from the bundle `cargo xtask bundle` makes",
        framework.display()
    );
}

/// The stage's handle to the browser. Cloning it shares the same browser.
#[derive(Clone)]
pub struct Browser {
    shared: SharedRef,
    /// Where CEF keeps this run's cache.
    cache: std::path::PathBuf,
    /// The port an agent connects to with the Chrome `DevTools` Protocol
    /// (CDP), if one was free.
    devtools_port: Option<u16>,
}

/// Starts CEF and the browser showing `url`, and returns the handle to it
/// with the events CEF will send.
///
/// Call this before anything else, on the main thread. On Linux, CEF's
/// helper processes (renderer, GPU, ...) run this same executable, and exit
/// inside this function.
///
/// # Panics
///
/// When CEF does not load or initialize.
#[must_use]
pub fn start(url: String) -> (Browser, UnboundedReceiver<Event>) {
    #[cfg(target_os = "macos")]
    load_framework(Process::Main);
    // The bindings were generated for this API version; tell CEF before
    // anything else.
    cef::api_hash(cef::sys::CEF_API_VERSION, 0);

    let initial = Viewport {
        width: 800,
        height: 600,
        scale: 1.0,
    };
    let shared = Arc::new(Mutex::new(Shared {
        wanted: initial,
        reported: initial,
        resizing: None,
        painted: None,
        frame: None,
        popup: None,
        popup_frame: None,
        host: None,
        caret: None,
        navigation: Navigation {
            url: url.clone(),
            ..Default::default()
        },
    }));
    let (events, receiver) = unbounded_channel();

    let args = Args::new();
    let command_line = args.as_cmd_line().expect("CEF reads the command line");
    let is_helper = command_line.has_switch(Some(&CefString::from("type"))) == 1;

    let mut app = handlers::StageApp::new(Arc::clone(&shared), events, url);
    let code = cef::execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if is_helper {
        std::process::exit(code);
    }
    assert_eq!(code, -1, "the browser process carries on");

    // A cache of its own per run: Chromium treats the cache directory as
    // the mark of a running instance, so two Cox windows must not share one.
    let cache = std::env::temp_dir().join(format!("cox-cef-{}", std::process::id()));
    let devtools_port = match free_port() {
        Ok(port) => Some(port),
        Err(error) => {
            log::error!("no port is free for DevTools, so agents cannot reach the stage: {error}");
            None
        }
    };
    let settings = Settings {
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
        // CEF runs its own UI thread, so GPUI can own the main thread.
        #[cfg(target_os = "linux")]
        multi_threaded_message_loop: 1,
        // macOS has no such option: CEF's UI thread is the main thread, and
        // the app gives it its turns with `Browser::pump`.
        #[cfg(target_os = "macos")]
        external_message_pump: 1,
        // 0 leaves the port closed.
        remote_debugging_port: devtools_port.map_or(0, i32::from),
        root_cache_path: CefString::from(cache.to_string_lossy().as_ref()),
        ..Default::default()
    };
    let initialized = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(initialized, 1, "CEF initializes");

    (
        Browser {
            shared,
            cache,
            devtools_port,
        },
        receiver,
    )
}

/// A port no one listens on right now. Chromium wants a port number, not
/// a listener, so another process could still take it before Chromium
/// does; the system hands out such ports in turn, which makes that rare.
fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

impl Browser {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        // A panic on CEF's thread leaves the state usable; carry on with it.
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn host(&self) -> Option<BrowserHost> {
        self.shared().host.clone()
    }

    /// The port an agent connects to with the Chrome `DevTools` Protocol
    /// (CDP), or `None` when none was free.
    #[must_use]
    pub fn devtools_port(&self) -> Option<u16> {
        self.devtools_port
    }

    /// Whether the browser exists and takes input.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.shared().host.is_some()
    }

    /// Tells CEF the stage's size in logical pixels and the window's scale
    /// factor. CEF hears of one size at a time: the next waits until a
    /// picture of this one came (see [`Browser::settle_resize`]).
    pub fn resize(&self, width: i32, height: i32, scale: f32) {
        self.shared().wanted = Viewport {
            width,
            height,
            scale,
        };
        self.settle_resize();
    }

    /// Moves a resize along: tells CEF of the stage's size once it painted
    /// the last size it was told. CEF does not always hand over the picture
    /// of a new size by itself, so after [`RESIZE_NUDGE`] it is asked for
    /// one, and when none of that size came within [`RESIZE_TIMEOUT`], the
    /// page is taken to be stuck at the size of its last picture: that size
    /// is reported and a picture asked for, so that the picture CEF waits
    /// for comes and the next size can be told. Call it now and then as
    /// well as on every resize.
    pub fn settle_resize(&self) {
        let mut shared = self.shared();
        let Some(host) = shared.host.clone() else {
            // Nothing has been told yet: the browser is created at this size.
            shared.reported = shared.wanted;
            return;
        };
        let now = Instant::now();
        let was = shared.reported;
        let ask = match shared.resizing {
            None if shared.wanted != shared.reported => {
                shared.reported = shared.wanted;
                shared.resizing = Some(Resizing {
                    since: now,
                    nudged: None,
                });
                Ask::Resize
            }
            Some(resizing) if now.duration_since(resizing.since) >= RESIZE_TIMEOUT => {
                let painted = shared.painted;
                log::debug!("no picture at {was:?} yet, painted {painted:?}");
                let Some(painted) = painted else {
                    return;
                };
                shared.reported = Viewport::painted_as(painted, was.scale);
                shared.resizing = Some(Resizing {
                    since: now,
                    nudged: None,
                });
                Ask::Picture
            }
            Some(resizing)
                if now.duration_since(resizing.nudged.unwrap_or(resizing.since))
                    >= RESIZE_NUDGE =>
            {
                shared.resizing = Some(Resizing {
                    nudged: Some(now),
                    ..resizing
                });
                Ask::Picture
            }
            _ => return,
        };
        let reported = shared.reported;
        // CEF answers these by asking the render handler for the screen
        // info, which locks `shared` too: on macOS that happens right here,
        // on this thread, so the lock must be gone by then.
        drop(shared);
        log::debug!("asking CEF: {ask:?} at {reported:?}");
        #[expect(clippy::float_cmp, reason = "the same value, or not")]
        if reported.scale != was.scale {
            host.notify_screen_info_changed();
        }
        match ask {
            Ask::Resize => host.was_resized(),
            Ask::Picture => host.invalidate(cef::PaintElementType::from(
                cef_paint_element_type_t::PET_VIEW,
            )),
        }
    }

    /// Gives CEF's UI thread a turn. Call it on the main thread, often: a
    /// few times per frame keeps the page responsive, and nothing happens on
    /// CEF's side between calls. Linux does not need it.
    #[cfg(target_os = "macos")]
    pub fn pump(&self) {
        crate::macos::cef_turn(cef::do_message_loop_work);
    }

    /// The newest picture CEF painted since the last call, if any.
    #[must_use]
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared().frame.take()
    }

    /// Where the browser is: its address and loading state.
    #[must_use]
    pub fn navigation(&self) -> Navigation {
        self.shared().navigation.clone()
    }

    fn browser(&self) -> Option<cef::Browser> {
        self.host().and_then(|host| host.browser())
    }

    /// Leaves the page for `url`.
    pub fn load(&self, url: &str) {
        if let Some(frame) = self.browser().and_then(|browser| browser.main_frame()) {
            frame.load_url(Some(&CefString::from(url)));
        }
    }

    pub fn go_back(&self) {
        if let Some(browser) = self.browser() {
            browser.go_back();
        }
    }

    pub fn go_forward(&self) {
        if let Some(browser) = self.browser() {
            browser.go_forward();
        }
    }

    pub fn reload(&self) {
        if let Some(browser) = self.browser() {
            browser.reload();
        }
    }

    pub fn stop(&self) {
        if let Some(browser) = self.browser() {
            browser.stop_load();
        }
    }

    /// Carries out `command` in the frame that has the focus.
    pub fn edit(&self, command: EditCommand) {
        let Some(frame) = self
            .browser()
            .and_then(|browser| browser.focused_frame().or_else(|| browser.main_frame()))
        else {
            return;
        };
        match command {
            EditCommand::Undo => frame.undo(),
            EditCommand::Redo => frame.redo(),
            EditCommand::Cut => frame.cut(),
            EditCommand::Copy => frame.copy(),
            EditCommand::Paste => frame.paste(),
            EditCommand::SelectAll => frame.select_all(),
        }
    }

    /// Where the popup is, while one is open.
    #[must_use]
    pub fn popup(&self) -> Option<PopupRect> {
        self.shared().popup
    }

    /// The newest picture CEF painted of the popup since the last call, if
    /// any.
    #[must_use]
    pub fn take_popup_frame(&self) -> Option<Frame> {
        self.shared().popup_frame.take()
    }

    /// Gives the page keyboard focus.
    pub fn focus(&self) {
        if let Some(host) = self.host() {
            host.set_focus(1);
        }
    }

    pub fn mouse_down(
        &self,
        x: i32,
        y: i32,
        button: MouseButton,
        clicks: i32,
        modifiers: Modifiers,
    ) {
        if let Some(host) = self.host() {
            let event = mouse_event(x, y, event_flags(modifiers) | button_flag(button));
            host.send_mouse_click_event(Some(&event), button_type(button), 0, clicks);
        }
    }

    pub fn mouse_up(&self, x: i32, y: i32, button: MouseButton, clicks: i32, modifiers: Modifiers) {
        if let Some(host) = self.host() {
            let event = mouse_event(x, y, event_flags(modifiers));
            host.send_mouse_click_event(Some(&event), button_type(button), 1, clicks);
        }
    }

    pub fn mouse_move(&self, x: i32, y: i32, pressed: Option<MouseButton>, modifiers: Modifiers) {
        if let Some(host) = self.host() {
            let flags = event_flags(modifiers) | pressed.map_or(0, button_flag);
            host.send_mouse_move_event(Some(&mouse_event(x, y, flags)), 0);
        }
    }

    /// Scrolls by `dx` and `dy` logical pixels at (`x`, `y`).
    pub fn wheel(&self, x: i32, y: i32, dx: i32, dy: i32, modifiers: Modifiers) {
        if let Some(host) = self.host() {
            let event = mouse_event(x, y, event_flags(modifiers));
            host.send_mouse_wheel_event(Some(&event), dx, dy);
        }
    }

    /// A key went down. `key` is GPUI's name for it ("a", "enter", ...).
    pub fn key_down(&self, key: &str, modifiers: Modifiers) {
        if let Some(host) = self.host() {
            let flags = event_flags(modifiers);
            host.send_key_event(Some(&key_event(KeyKind::Down, key, flags, 0)));
            // Printable characters arrive through the input handler (see
            // `commit`); Enter and Tab are not printable but the page still
            // expects a character for them.
            let control_char = match key {
                "enter" => Some(u16::from(b'\r')),
                "tab" => Some(u16::from(b'\t')),
                _ => None,
            };
            if let Some(character) = control_char {
                host.send_key_event(Some(&key_event(KeyKind::Char, key, flags, character)));
            }
        }
    }

    pub fn key_up(&self, key: &str, modifiers: Modifiers) {
        if let Some(host) = self.host() {
            let event = key_event(KeyKind::Up, key, event_flags(modifiers), 0);
            host.send_key_event(Some(&event));
        }
    }

    /// Final text: a typed character, or a composition the input method
    /// committed. A single character goes in as a key event, so the page
    /// sees `keypress` as it would in a browser.
    pub fn commit(&self, text: &str) {
        let Some(host) = self.host() else { return };
        let mut units = text.encode_utf16();
        match (units.next(), units.next()) {
            (Some(unit), None) if !text.chars().all(char::is_control) => {
                host.send_key_event(Some(&key_event(KeyKind::Char, text, 0, unit)));
            }
            _ => host.ime_commit_text(Some(&CefString::from(text)), Some(&NO_RANGE), 0),
        }
    }

    /// Text the input method is composing, shown underlined in the page,
    /// with the caret at `selection` (UTF-16 offsets within `text`).
    pub fn compose(&self, text: &str, selection: Option<std::ops::Range<usize>>) {
        let Some(host) = self.host() else { return };
        let len = utf16_len(text);
        let underline = CompositionUnderline {
            range: cef::Range { from: 0, to: len },
            color: 0xFF00_0000,
            background_color: 0,
            thick: 0,
            ..Default::default()
        };
        let selection = selection.map_or(cef::Range { from: len, to: len }, |range| cef::Range {
            from: u32::try_from(range.start).unwrap_or(len),
            to: u32::try_from(range.end).unwrap_or(len),
        });
        host.ime_set_composition(
            Some(&CefString::from(text)),
            Some(&[underline]),
            Some(&NO_RANGE),
            Some(&selection),
        );
    }

    /// The input method stopped composing: what it composed stays as
    /// regular text.
    pub fn finish_composition(&self) {
        if let Some(host) = self.host() {
            host.ime_finish_composing_text(0);
        }
    }

    /// Where the page says the composition caret is, for placing the input
    /// method's candidate window.
    #[must_use]
    pub fn caret(&self) -> Option<Caret> {
        self.shared().caret
    }

    /// Closes the browser, waits for CEF to let go of it, and shuts CEF
    /// down. Call once GPUI is done, on the main thread.
    pub fn shutdown(self) {
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.is_open() && Instant::now() < deadline {
            // Closing happens on CEF's UI thread, which on macOS is this one.
            #[cfg(target_os = "macos")]
            self.pump();
            std::thread::sleep(Duration::from_millis(50));
        }
        cef::shutdown();
        if let Err(error) = std::fs::remove_dir_all(&self.cache) {
            log::warn!("the cache at {} stays: {error}", self.cache.display());
        }
    }
}

fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}

fn mouse_event(x: i32, y: i32, modifiers: u32) -> MouseEvent {
    MouseEvent { x, y, modifiers }
}

/// Modifier bits CEF expects on mouse and key events.
fn event_flags(modifiers: Modifiers) -> u32 {
    let mut flags = 0;
    if modifiers.shift {
        flags |= cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0;
    }
    if modifiers.control {
        flags |= cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0;
    }
    if modifiers.alt {
        flags |= cef_event_flags_t::EVENTFLAG_ALT_DOWN.0;
    }
    if modifiers.command {
        flags |= cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0;
    }
    flags
}

fn button_flag(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON.0,
        MouseButton::Middle => cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0,
        MouseButton::Right => cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON.0,
    }
}

fn button_type(button: MouseButton) -> MouseButtonType {
    MouseButtonType::from(match button {
        MouseButton::Left => cef_mouse_button_type_t::MBT_LEFT,
        MouseButton::Middle => cef_mouse_button_type_t::MBT_MIDDLE,
        MouseButton::Right => cef_mouse_button_type_t::MBT_RIGHT,
    })
}

/// Windows virtual key code for a GPUI key name: CEF speaks VK codes on
/// every platform. GPUI names a key by what it types without Shift, on
/// the US layout for the symbols. Keys without a code here still type,
/// since their character arrives through [`Browser::commit`].
fn virtual_key(key: &str) -> i32 {
    match key {
        "backspace" => 0x08,
        "tab" => 0x09,
        "enter" => 0x0D,
        "shift" => 0x10,
        "control" => 0x11,
        "alt" => 0x12,
        "capslock" => 0x14,
        "escape" => 0x1B,
        "space" => 0x20,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "end" => 0x23,
        "home" => 0x24,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "insert" => 0x2D,
        "delete" => 0x2E,
        "platform" => 0x5B,
        ";" => 0xBA,
        "=" => 0xBB,
        "," => 0xBC,
        "-" => 0xBD,
        "." => 0xBE,
        "/" => 0xBF,
        "`" => 0xC0,
        "[" => 0xDB,
        "\\" => 0xDC,
        "]" => 0xDD,
        "'" => 0xDE,
        key => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => {
                    i32::from(c.to_ascii_uppercase() as u8)
                }
                // f1 to f24.
                (Some('f'), Some(_)) => key[1..]
                    .parse::<i32>()
                    .ok()
                    .filter(|n| (1..=24).contains(n))
                    .map_or(0, |n| 0x6F + n),
                _ => 0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Viewport;
    use super::virtual_key;

    #[test]
    fn keys_have_their_windows_codes() {
        assert_eq!(virtual_key("a"), 0x41);
        assert_eq!(virtual_key("7"), 0x37);
        assert_eq!(virtual_key("enter"), 0x0D);
        assert_eq!(virtual_key("-"), 0xBD);
        assert_eq!(virtual_key("f1"), 0x70);
        assert_eq!(virtual_key("f12"), 0x7B);
    }

    #[test]
    fn unknown_keys_have_no_code() {
        assert_eq!(virtual_key("f0"), 0);
        assert_eq!(virtual_key("fn"), 0);
        assert_eq!(virtual_key("ä"), 0);
    }

    #[test]
    fn device_size_rounds_up() {
        let viewport = Viewport {
            width: 635,
            height: 400,
            scale: 1.5,
        };
        assert_eq!(viewport.device_size(), (953, 600));
    }

    #[test]
    fn painted_as_finds_the_viewport_behind_a_picture() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for logical in [1, 2, 3, 635, 636, 888, 1113] {
                let viewport = Viewport {
                    width: logical,
                    height: logical,
                    scale,
                };
                let back = Viewport::painted_as(viewport.device_size(), scale);
                assert_eq!(back.device_size(), viewport.device_size(), "{viewport:?}");
            }
        }
    }
}
#[derive(Clone, Copy)]
enum KeyKind {
    Down,
    Up,
    Char,
}

fn key_event(kind: KeyKind, key: &str, modifiers: u32, character: u16) -> KeyEvent {
    let type_ = match kind {
        KeyKind::Down => cef_key_event_type_t::KEYEVENT_RAWKEYDOWN,
        KeyKind::Up => cef_key_event_type_t::KEYEVENT_KEYUP,
        KeyKind::Char => cef_key_event_type_t::KEYEVENT_CHAR,
    };
    KeyEvent {
        type_: KeyEventType::from(type_),
        modifiers,
        windows_key_code: virtual_key(key),
        native_key_code: 0,
        is_system_key: 0,
        character,
        unmodified_character: character,
        focus_on_editable_field: 0,
        ..Default::default()
    }
}

/// CEF's side of the stage: what CEF calls into on its own threads.
mod handlers {
    #![expect(
        clippy::transmute_ptr_to_ptr,
        reason = "the cef wrap macros cast the handler reference CEF passes back"
    )]

    use super::*;

    wrap_app! {
        pub(super) struct StageApp {
            shared: SharedRef,
            events: UnboundedSender<Event>,
            url: String,
        }

        impl App {
            /// Chromium's first-run flow would open a terms-of-service window
            /// of its own and wait for it; Cox is not that kind of browser.
            /// On macOS it would also ask for the login keychain, to keep the
            /// key it encrypts cookies with; a cache that lives for one run
            /// has no use for that.
            ///
            /// Going back would bring a page out of the back-forward cache
            /// frozen at the size it was left at, and painted so until the
            /// renderer gets around to the stage's size, seconds later;
            /// loading the page again lays it out at the right size at once.
            fn on_before_command_line_processing(
                &self,
                _process_type: Option<&CefString>,
                command_line: Option<&mut CommandLine>,
            ) {
                if let Some(command_line) = command_line {
                    command_line.append_switch(Some(&CefString::from("no-first-run")));
                    command_line.append_switch_with_value(
                        Some(&CefString::from("disable-features")),
                        Some(&CefString::from("BackForwardCache")),
                    );
                    #[cfg(target_os = "macos")]
                    command_line.append_switch(Some(&CefString::from("use-mock-keychain")));
                }
            }

            fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
                Some(StageProcessHandler::new(
                    Arc::clone(&self.shared),
                    self.events.clone(),
                    self.url.clone(),
                ))
            }
        }
    }

    wrap_browser_process_handler! {
        struct StageProcessHandler {
            shared: SharedRef,
            events: UnboundedSender<Event>,
            url: String,
        }

        impl BrowserProcessHandler {
            /// CEF is ready: create the windowless browser.
            fn on_context_initialized(&self) {
                let (width, height) = {
                    let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                    (shared.reported.width, shared.reported.height)
                };
                let window_info = WindowInfo {
                    windowless_rendering_enabled: 1,
                    bounds: cef::Rect { x: 0, y: 0, width, height },
                    ..Default::default()
                };
                let mut client = StageClient::new(
                    StageRenderHandler::new(Arc::clone(&self.shared), self.events.clone()),
                    StageLifeSpanHandler::new(Arc::clone(&self.shared), self.events.clone()),
                    StageLoadHandler::new(Arc::clone(&self.shared), self.events.clone()),
                    StageDisplayHandler::new(Arc::clone(&self.shared), self.events.clone()),
                );
                let settings = BrowserSettings {
                    windowless_frame_rate: 60,
                    ..Default::default()
                };
                let url = CefString::from(self.url.as_str());
                let created = cef::browser_host_create_browser(
                    Some(&window_info),
                    Some(&mut client),
                    Some(&url),
                    Some(&settings),
                    None,
                    None,
                );
                if created != 1 {
                    log::error!("CEF did not create the browser");
                }
            }
        }
    }

    wrap_client! {
        struct StageClient {
            render: RenderHandler,
            life_span: LifeSpanHandler,
            load: LoadHandler,
            display: DisplayHandler,
        }

        impl Client {
            fn render_handler(&self) -> Option<RenderHandler> {
                Some(self.render.clone())
            }

            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(self.life_span.clone())
            }

            fn load_handler(&self) -> Option<LoadHandler> {
                Some(self.load.clone())
            }

            fn display_handler(&self) -> Option<DisplayHandler> {
                Some(self.display.clone())
            }
        }
    }

    wrap_load_handler! {
        struct StageLoadHandler {
            shared: SharedRef,
            events: UnboundedSender<Event>,
        }

        impl LoadHandler {
            /// Loading started or finished somewhere in the page, and the
            /// history may have moved.
            fn on_loading_state_change(
                &self,
                _browser: Option<&mut cef::Browser>,
                is_loading: c_int,
                can_go_back: c_int,
                can_go_forward: c_int,
            ) {
                {
                    let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                    shared.navigation.loading = is_loading == 1;
                    shared.navigation.can_go_back = can_go_back == 1;
                    shared.navigation.can_go_forward = can_go_forward == 1;
                }
                let _ = self.events.send(Event::Navigated);
            }
        }
    }

    wrap_display_handler! {
        struct StageDisplayHandler {
            shared: SharedRef,
            events: UnboundedSender<Event>,
        }

        impl DisplayHandler {
            /// The page's address changed. Frames inside the page have
            /// addresses too; only the page's own counts.
            fn on_address_change(
                &self,
                _browser: Option<&mut cef::Browser>,
                frame: Option<&mut cef::Frame>,
                url: Option<&CefString>,
            ) {
                let is_main = frame.is_some_and(|frame| frame.is_main() == 1);
                if let (true, Some(url)) = (is_main, url) {
                    self.shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .navigation
                        .url = url.to_string();
                    let _ = self.events.send(Event::Navigated);
                }
            }
        }
    }

    wrap_life_span_handler! {
        struct StageLifeSpanHandler {
            shared: SharedRef,
            events: UnboundedSender<Event>,
        }

        impl LifeSpanHandler {
            /// The page wants a window of its own, for a link with
            /// `target="_blank"` or `window.open`. The stage is the only
            /// window there is, so the page goes there instead.
            fn on_before_popup(
                &self,
                browser: Option<&mut cef::Browser>,
                _frame: Option<&mut cef::Frame>,
                _popup_id: c_int,
                target_url: Option<&CefString>,
                _target_frame_name: Option<&CefString>,
                _target_disposition: cef::WindowOpenDisposition,
                _user_gesture: c_int,
                _popup_features: Option<&cef::PopupFeatures>,
                _window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>,
                _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<cef::DictionaryValue>>,
                _no_javascript_access: Option<&mut c_int>,
            ) -> c_int {
                let frame = browser.and_then(|browser| browser.main_frame());
                match (frame, target_url) {
                    (Some(frame), Some(url)) if !url.to_string().is_empty() => {
                        frame.load_url(Some(url));
                    }
                    // `window.open()` with no address opens a blank page
                    // for the script to fill; there is nowhere to show it.
                    _ => {}
                }
                // 1: no popup.
                1
            }

            fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
                if let Some(browser) = browser {
                    let host = browser.host();
                    if let Some(host) = &host {
                        host.was_resized();
                        host.set_focus(1);
                    }
                    self.shared.lock().unwrap_or_else(PoisonError::into_inner).host = host;
                    let _ = self.events.send(Event::Created);
                }
            }

            fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
                self.shared.lock().unwrap_or_else(PoisonError::into_inner).host = None;
                let _ = self.events.send(Event::Closed);
            }
        }
    }

    wrap_render_handler! {
        struct StageRenderHandler {
            shared: SharedRef,
            events: UnboundedSender<Event>,
        }

        impl RenderHandler {
            /// CEF asks how big the view is, in logical pixels.
            fn view_rect(&self, _browser: Option<&mut cef::Browser>, rect: Option<&mut cef::Rect>) {
                let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(rect) = rect {
                    *rect = cef::Rect {
                        x: 0,
                        y: 0,
                        width: shared.reported.width.max(1),
                        height: shared.reported.height.max(1),
                    };
                }
            }

            /// CEF asks about the screen: the scale factor decides how many
            /// device pixels it paints per logical pixel.
            fn screen_info(
                &self,
                _browser: Option<&mut cef::Browser>,
                screen_info: Option<&mut cef::ScreenInfo>,
            ) -> c_int {
                let shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(info) = screen_info {
                    let rect = cef::Rect {
                        x: 0,
                        y: 0,
                        width: shared.reported.width.max(1),
                        height: shared.reported.height.max(1),
                    };
                    info.device_scale_factor = shared.reported.scale;
                    info.rect = rect.clone();
                    info.available_rect = rect;
                }
                1
            }

            /// CEF painted the view or the popup: keep the picture and wake
            /// the stage.
            fn on_paint(
                &self,
                _browser: Option<&mut cef::Browser>,
                type_: cef::PaintElementType,
                _dirty_rects: Option<&[cef::Rect]>,
                buffer: *const u8,
                width: c_int,
                height: c_int,
            ) {
                if buffer.is_null() {
                    return;
                }
                let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
                    return;
                };
                let len = width as usize * height as usize * 4;
                // SAFETY: CEF gives `width * height` BGRA pixels, valid for the
                // duration of this call; the copy outlives it.
                let bgra = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
                let frame = Some(Frame {
                    width,
                    height,
                    bgra,
                });
                let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                match *type_.as_ref() {
                    cef_paint_element_type_t::PET_VIEW => {
                        shared.painted = Some((width, height));
                        if shared.resizing.is_some()
                            && shared.reported.device_size() == (width, height)
                        {
                            // The picture CEF held its others back for: the
                            // stage's next paint tells it of the next size,
                            // if any.
                            shared.resizing = None;
                        }
                        shared.frame = frame;
                    }
                    cef_paint_element_type_t::PET_POPUP => shared.popup_frame = frame,
                    _ => return,
                }
                drop(shared);
                let _ = self.events.send(Event::Painted);
            }

            /// The popup opened or closed. Its place comes separately, in
            /// `on_popup_size`, before the first paint.
            fn on_popup_show(&self, _browser: Option<&mut cef::Browser>, show: c_int) {
                if show == 0 {
                    let mut shared = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
                    shared.popup = None;
                    shared.popup_frame = None;
                    drop(shared);
                    let _ = self.events.send(Event::Painted);
                }
            }

            fn on_popup_size(&self, _browser: Option<&mut cef::Browser>, rect: Option<&cef::Rect>) {
                if let Some(rect) = rect {
                    self.shared.lock().unwrap_or_else(PoisonError::into_inner).popup =
                        Some(PopupRect {
                            x: rect.x,
                            y: rect.y,
                            width: rect.width,
                            height: rect.height,
                        });
                }
            }

            /// Where the composition is on the page, for placing the input
            /// method's candidate window.
            fn on_ime_composition_range_changed(
                &self,
                _browser: Option<&mut cef::Browser>,
                _selected_range: Option<&cef::Range>,
                character_bounds: Option<&[cef::Rect]>,
            ) {
                let caret = character_bounds.and_then(<[_]>::first).map(|rect| Caret {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                });
                self.shared.lock().unwrap_or_else(PoisonError::into_inner).caret = caret;
            }
        }
    }
}
