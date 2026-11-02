//! The browsers behind the stages: CEF (Chromium) rendering pages with no
//! window of their own. [`Cef`] runs once for the whole app, and opens a
//! [`Browser`] for each chat's stage. CEF hands each painted picture to
//! the browser's [`Shared`], and the stage hands mouse, keys and
//! input-method text back through the [`Browser`]. CEF's remote debugging
//! port stays on, so agents can drive the same pages over CDP meanwhile.
//! The port is one that is free when Cox starts, so that two Cox can run
//! side by side.
//!
//! CEF runs its own threads: the browser is created and painted on CEF's UI
//! thread, while GPUI owns the main thread. Everything the two sides share
//! sits behind one mutex. On macOS, CEF's UI thread has to be the main
//! thread, so GPUI gives it a turn every few milliseconds instead (see
//! [`Browser::pump`]).
//!
//! - [`relay`] stands between a chat's agent's browser tools and the
//!   browser, keeps the agent to its chat's pages, opens and closes tabs
//!   for it, and tells the stage where the agent points, clicks and
//!   types, which [`pointer`] turns into what the tab draws over the page.
//! - [`profile`] is where the browser keeps logins and the like.
//! - [`handlers`] is what CEF calls into on its threads, [`viewport`]
//!   fits the page to the stage's size, [`events`] makes CEF's mouse
//!   and key events of the stage's, and [`links`] tells which of a
//!   page's navigations is a link to a file.
//! - [`scheme`] serves the `cox:` addresses a stage's pages read what Cox
//!   knows from, as the stage [serves](scheme::Serve) them.
//! - [`quit`] lets signals, and CEF on macOS, ask the app to quit, and
//!   [`macos`] gives CEF what it expects of the app there.

mod events;
mod handlers;
mod links;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod pointer;
pub mod profile;
pub mod quit;
pub mod relay;
pub mod scheme;
mod viewport;

use std::os::raw::c_int;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use cef::args::Args;
use cef::sys::{cef_paint_element_type_t, cef_thread_id_t};
// The `wrap_*` macros name these without a path, so they have to be in scope.
use cef::rc::Rc as _;
use cef::{
    App, BrowserHost, BrowserProcessHandler, BrowserSettings, CefString, Client, CommandLine,
    CompositionUnderline, DevToolsMessageObserver, DisplayHandler, ImplApp, ImplBrowser,
    ImplBrowserHost, ImplBrowserProcessHandler, ImplClient, ImplCommandLine,
    ImplDevToolsMessageObserver, ImplDisplayHandler, ImplFrame, ImplLifeSpanHandler,
    ImplLoadHandler, ImplRenderHandler, ImplRequest, ImplRequestHandler, ImplTask, LifeSpanHandler,
    LoadHandler, Registration, RenderHandler, RequestHandler, ResourceRequestHandler,
    SchemeRegistrar, Settings, Task, ThreadId, WindowInfo, WrapApp, WrapBrowserProcessHandler,
    WrapClient, WrapDevToolsMessageObserver, WrapDisplayHandler, WrapLifeSpanHandler,
    WrapLoadHandler, WrapRenderHandler, WrapRequestHandler, WrapTask, wrap_app,
    wrap_browser_process_handler, wrap_client, wrap_dev_tools_message_observer,
    wrap_display_handler, wrap_life_span_handler, wrap_load_handler, wrap_render_handler,
    wrap_request_handler, wrap_task,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

use crate::events::{
    KeyKind, button_flag, button_type, event_flags, key_character, key_event, mouse_event,
};
use crate::profile::Profile;
use crate::viewport::{Resizing, Viewport};

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
    /// The page moved, started or finished loading, or changed its title:
    /// see [`Browser::navigation`].
    Navigated,
    /// The page asks for a window of its own, showing this address.
    Popup(String),
    /// The human clicked a link in the page to a file, at this `file:`
    /// address, with lines after it if it names any (see [`links`]), or
    /// the page went to a `cox://open/` address for it (see [`scheme`]):
    /// the page stays, for the stage to open the file on a tab of its own.
    FileLink(String),
}

/// Where the browser is in its history, as the toolbar shows it, and what
/// the page is called, as its tab shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Navigation {
    /// The address of the page being shown, or being loaded.
    pub url: String,
    /// The page's title, empty until it has one.
    pub title: String,
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
    /// Keeps CEF telling the browser's `DevTools` answers to the observer
    /// that learns the page's target id.
    devtools: Option<Registration>,
    caret: Option<Caret>,
    navigation: Navigation,
}

impl Shared {
    fn new(url: &str) -> Self {
        // CEF is told the stage's size once it is laid out.
        let initial = Viewport {
            width: 800,
            height: 600,
            scale: 1.0,
        };
        Self {
            wanted: initial,
            reported: initial,
            resizing: None,
            painted: None,
            frame: None,
            popup: None,
            popup_frame: None,
            host: None,
            devtools: None,
            caret: None,
            navigation: Navigation {
                url: url.to_owned(),
                ..Default::default()
            },
        }
    }
}

type SharedRef = Arc<Mutex<Shared>>;

/// The id `DevTools` knows a page by, as a target of the Chrome `DevTools`
/// Protocol, once CEF has told it: `None` until then.
pub type TargetId = watch::Receiver<Option<String>>;

/// What it takes to create a browser on CEF's UI thread.
struct NewBrowser {
    shared: SharedRef,
    events: UnboundedSender<Event>,
    target: Arc<watch::Sender<Option<String>>>,
    url: String,
    /// What its pages get at `cox:` addresses, if anything.
    serve: Option<scheme::Serve>,
}

/// The browsers of the app, as far as CEF's start and end are concerned.
#[derive(Default)]
struct Browsers {
    /// Whether CEF is ready to create browsers.
    ready: bool,
    /// The browsers asked for before then.
    waiting: Vec<NewBrowser>,
    /// Every browser that is still around, to close them all when Cox
    /// quits.
    all: Vec<Weak<Mutex<Shared>>>,
}

type BrowsersRef = Arc<Mutex<Browsers>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic on CEF's thread leaves the state usable; carry on with it.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// CEF's "no range" (`CefRange::InvalidRange()`): the C API ignores a null
/// range, so "replace nothing" has to be spelled out.
const NO_RANGE: cef::Range = cef::Range {
    from: u32::MAX,
    to: u32::MAX,
};

/// Which process of the app is loading CEF on macOS.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Process {
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
fn load_framework(process: Process) {
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

/// Runs a helper process of the app on macOS, the work CEF gives it on
/// the command line, and returns its exit code. Each helper app under
/// `Cox.app/Contents/Frameworks` does only this.
///
/// # Panics
///
/// When this executable is not inside the bundle `cargo xtask bundle`
/// makes, or the framework does not load.
#[cfg(target_os = "macos")]
#[must_use]
pub fn run_helper() -> i32 {
    load_framework(Process::Helper);
    cef::api_hash(cef::sys::CEF_API_VERSION, 0);
    let args = Args::new();
    cef::execute_process(
        Some(args.as_main_args()),
        None::<&mut cef::App>,
        std::ptr::null_mut(),
    )
}

/// CEF, running for the whole app. Cloning it shares the same CEF.
#[derive(Clone)]
pub struct Cef {
    browsers: BrowsersRef,
    /// Where CEF keeps cookies, logins and the like, for every browser.
    profile: Arc<Profile>,
    /// The port agents connect to with the Chrome `DevTools` Protocol (CDP),
    /// if one was free.
    devtools_port: Option<u16>,
}

/// A stage's handle to its browser. Cloning it shares the same browser.
#[derive(Clone)]
pub struct Browser {
    shared: SharedRef,
    target: TargetId,
}

/// Starts CEF, and returns the handle to it.
///
/// Call this before anything else, on the main thread. On Linux, CEF's
/// helper processes (renderer, GPU, ...) run this same executable, and exit
/// inside this function.
///
/// # Panics
///
/// When CEF does not load or initialize.
#[must_use]
pub fn start(profile: Option<PathBuf>) -> Cef {
    #[cfg(target_os = "macos")]
    load_framework(Process::Main);
    // The bindings were generated for this API version; tell CEF before
    // anything else.
    cef::api_hash(cef::sys::CEF_API_VERSION, 0);

    let browsers = BrowsersRef::default();

    let args = Args::new();
    let command_line = args.as_cmd_line().expect("CEF reads the command line");
    let is_helper = command_line.has_switch(Some(&CefString::from("type"))) == 1;

    let mut app = handlers::StageApp::new(Arc::clone(&browsers));
    let code = cef::execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if is_helper {
        std::process::exit(code);
    }
    assert_eq!(code, -1, "the browser process carries on");

    let profile = Profile::open_at(profile);
    let profile_path = CefString::from(profile.path().to_string_lossy().as_ref());
    let devtools_port = match free_port() {
        Ok(port) => Some(port),
        Err(error) => {
            tracing::error!(
                "no port is free for DevTools, so agents cannot reach the stage: {error}"
            );
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
        root_cache_path: profile_path.clone(),
        // Left empty, CEF keeps cookies and storage in memory only, which
        // a throwaway profile may as well.
        cache_path: if profile.is_kept() {
            profile_path
        } else {
            CefString::default()
        },
        ..Default::default()
    };
    let initialized = cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    assert_eq!(initialized, 1, "CEF initializes");

    Cef {
        browsers,
        profile: Arc::new(profile),
        devtools_port,
    }
}

/// A port no one listens on right now. Chromium wants a port number, not
/// a listener, so another process could still take it before Chromium
/// does; the system hands out such ports in turn, which makes that rare.
fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

impl Cef {
    /// The port agents connect to with the Chrome `DevTools` Protocol
    /// (CDP), or `None` when none was free.
    #[must_use]
    pub fn devtools_port(&self) -> Option<u16> {
        self.devtools_port
    }

    /// Opens a browser showing `url`, whose pages get what `serve` answers
    /// at `cox:` addresses (see [`scheme`]), and returns the handle to it
    /// with the events CEF will send. The browser lives until
    /// [`Browser::close`].
    #[must_use]
    pub fn open(
        &self,
        url: &str,
        serve: Option<scheme::Serve>,
    ) -> (Browser, UnboundedReceiver<Event>) {
        let shared = Arc::new(Mutex::new(Shared::new(url)));
        let (events, receiver) = unbounded_channel();
        let (target, target_id) = watch::channel(None);
        let new = NewBrowser {
            shared: Arc::clone(&shared),
            events,
            target: Arc::new(target),
            url: url.to_owned(),
            serve,
        };
        let mut browsers = lock(&self.browsers);
        browsers.all.retain(|browser| browser.strong_count() > 0);
        browsers.all.push(Arc::downgrade(&shared));
        if browsers.ready {
            drop(browsers);
            // CEF creates browsers on its UI thread.
            let mut task = handlers::CreateBrowser::new(Arc::new(Mutex::new(Some(new))));
            cef::post_task(ThreadId::from(cef_thread_id_t::TID_UI), Some(&mut task));
        } else {
            browsers.waiting.push(new);
        }
        (
            Browser {
                shared,
                target: target_id,
            },
            receiver,
        )
    }

    /// Gives CEF's UI thread a turn. Call it on the main thread, often: a
    /// few times per frame keeps the pages responsive, and nothing happens
    /// on CEF's side between calls. Linux does not need it.
    #[cfg(target_os = "macos")]
    pub fn pump(&self) {
        crate::macos::cef_turn(cef::do_message_loop_work);
    }

    /// Closes every browser, waits for CEF to let go of them, and shuts
    /// CEF down. Call once GPUI is done, on the main thread.
    pub fn shutdown(self) {
        let open: Vec<SharedRef> = lock(&self.browsers)
            .all
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let host = |shared: &SharedRef| lock(shared).host.clone();
        for host in open.iter().filter_map(host) {
            host.close_browser(1);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while open.iter().any(|shared| host(shared).is_some()) && Instant::now() < deadline {
            // Closing happens on CEF's UI thread, which on macOS is this one.
            #[cfg(target_os = "macos")]
            self.pump();
            std::thread::sleep(Duration::from_millis(50));
        }
        // A browser that did not close in time is past using.
        for shared in &open {
            lock(shared).host = None;
        }
        cef::shutdown();
        self.profile.discard();
    }
}

impl Browser {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        lock(&self.shared)
    }

    fn host(&self) -> Option<BrowserHost> {
        self.shared().host.clone()
    }

    /// The id `DevTools` knows the page by, once CEF has told it.
    #[must_use]
    pub fn target_id(&self) -> TargetId {
        self.target.clone()
    }

    /// Whether the browser exists and takes input.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.shared().host.is_some()
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
            let event = key_event(KeyKind::Down, key, flags, key_character(key));
            host.send_key_event(Some(&event));
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
            let event = key_event(KeyKind::Up, key, event_flags(modifiers), key_character(key));
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

    /// Closes the browser. CEF lets go of it on its own time.
    pub fn close(&self) {
        if let Some(host) = self.host() {
            host.close_browser(1);
        }
    }
}

fn utf16_len(text: &str) -> u32 {
    u32::try_from(text.encode_utf16().count()).unwrap_or(u32::MAX)
}
