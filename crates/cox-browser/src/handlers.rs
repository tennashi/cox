//! CEF's side of the stage: what CEF calls into on its own threads,
//! and what it is told there.
#![expect(
    clippy::transmute_ptr_to_ptr,
    reason = "the cef wrap macros cast the handler reference CEF passes back"
)]

use super::*;

wrap_app! {
    pub(super) struct StageApp {
        browsers: BrowsersRef,
    }

    impl App {
        /// Chromium's first-run flow would open a terms-of-service window
        /// of its own and wait for it; Cox is not that kind of browser.
        /// On macOS it would also ask for the login keychain, to keep the
        /// key it encrypts cookies with, and ask again for each build of
        /// Cox, which the keychain takes for a different app. So the
        /// cookies Cox keeps are guarded by the permissions on their
        /// directory alone.
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
            Some(StageProcessHandler::new(Arc::clone(&self.browsers)))
        }

        /// Each process of CEF learns the `cox:` scheme the stage serves
        /// (see [`crate::scheme`]).
        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            if let Some(registrar) = registrar {
                crate::scheme::register(*registrar);
            }
        }
    }
}

wrap_browser_process_handler! {
    struct StageProcessHandler {
        browsers: BrowsersRef,
    }

    impl BrowserProcessHandler {
        /// CEF is ready: create the browsers asked for so far.
        fn on_context_initialized(&self) {
            let waiting = {
                let mut browsers = lock(&self.browsers);
                browsers.ready = true;
                std::mem::take(&mut browsers.waiting)
            };
            for new in waiting {
                create(new);
            }
        }

        /// CEF has work for its UI thread in `delay_ms`. On macOS, where
        /// that is the main thread, work wanted now wakes its turn; the
        /// turns GPUI gives it anyway come soon enough for the rest. On
        /// Linux, CEF's thread runs its own loop.
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            #[cfg(target_os = "macos")]
            if delay_ms <= 0 {
                crate::macos::want_pump();
            }
            #[cfg(not(target_os = "macos"))]
            let _ = delay_ms;
        }
    }
}

wrap_task! {
    pub(super) struct CreateBrowser {
        // A task can be copied; only the first run creates the browser.
        new: Arc<Mutex<Option<NewBrowser>>>,
    }

    impl Task {
        fn execute(&self) {
            if let Some(new) = lock(&self.new).take() {
                create(new);
            }
        }
    }
}

wrap_task! {
    struct CloseBrowser {
        host: BrowserHost,
    }

    impl Task {
        fn execute(&self) {
            self.host.close_browser(1);
        }
    }
}

/// Creates the windowless browser `new` asks for. Call it on CEF's UI
/// thread.
fn create(new: NewBrowser) {
    let NewBrowser {
        shared,
        events,
        target,
        url,
        serve,
    } = new;
    let (width, height) = {
        let shared = lock(&shared);
        (shared.reported.width, shared.reported.height)
    };
    let window_info = WindowInfo {
        windowless_rendering_enabled: 1,
        // CEF draws a picture when it is asked for one (see `frames`).
        external_begin_frame_enabled: 1,
        bounds: cef::Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
        ..Default::default()
    };
    let mut client = StageClient::new(
        StageRenderHandler::new(Arc::clone(&shared), events.clone()),
        StageLifeSpanHandler::new(Arc::clone(&shared), events.clone(), target),
        StageLoadHandler::new(Arc::clone(&shared), events.clone()),
        StageDisplayHandler::new(Arc::clone(&shared), events.clone()),
        StageRequestHandler::new(events.clone(), serve),
        StageContextMenuHandler::new(events),
    );
    let settings = BrowserSettings {
        // CEF hands over at most this many pictures a second, whatever
        // asks for them: above any display's rate, so that the window's
        // frames alone pace them (see `frames`).
        windowless_frame_rate: 240,
        // White behind a page that sets no background, as the settings
        // CEF starts with have it (see `start`).
        background_color: 0xFFFF_FFFF,
        ..Default::default()
    };
    let url = CefString::from(url.as_str());
    let created = cef::browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url),
        Some(&settings),
        None,
        None,
    );
    if created != 1 {
        tracing::error!("CEF did not create a browser");
    }
}

wrap_dev_tools_message_observer! {
    struct TargetObserver {
        target: Arc<watch::Sender<Option<String>>>,
    }

    impl DevToolsMessageObserver {
        /// The answer to `Target.getTargetInfo`, the only method Cox
        /// calls through CEF, which names the page's target.
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            _message_id: c_int,
            success: c_int,
            result: Option<&[u8]>,
        ) {
            let target = result
                .filter(|_| success == 1)
                .and_then(|result| serde_json::from_slice::<serde_json::Value>(result).ok())
                .and_then(|result| result["targetInfo"]["targetId"].as_str().map(str::to_owned));
            if let Some(target) = target {
                self.target.send_replace(Some(target));
            } else {
                tracing::error!("CEF did not name the page's target, so its agent cannot reach it");
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
        request: RequestHandler,
        context_menu: ContextMenuHandler,
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

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.request.clone())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.context_menu.clone())
        }
    }
}

wrap_context_menu_handler! {
    struct StageContextMenuHandler {
        events: UnboundedSender<Event>,
    }

    impl ContextMenuHandler {
        /// The human pressed the right button in the page. The browser has
        /// no window to show its menu in, so it shows none; when text is
        /// selected, the stage offers what to do with it instead.
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            if let Some(model) = model {
                model.clear();
            }
            let Some(params) = params else {
                return;
            };
            let text = CefString::from(&params.selection_text()).to_string();
            if !text.trim().is_empty() {
                let _ = self.events.send(Event::Selected {
                    x: params.xcoord(),
                    y: params.ycoord(),
                    text,
                });
            }
        }
    }
}

wrap_request_handler! {
    struct StageRequestHandler {
        events: UnboundedSender<Event>,
        // What the pages get at `cox:` addresses, if anything.
        serve: Option<crate::scheme::Serve>,
    }

    impl RequestHandler {
        /// The page is about to go somewhere. When that is a link to a
        /// file the human clicked (see [`crate::links`]), or a `cox://open/`
        /// address for a file (see [`crate::scheme`]), the page stays and
        /// the stage opens the file on a tab of its own; when it is a
        /// `cox://chat/` address, the page stays and the window shows the
        /// chat.
        fn on_before_browse(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            user_gesture: c_int,
            _is_redirect: c_int,
        ) -> c_int {
            let is_main = frame.is_some_and(|frame| frame.is_main() == 1);
            let url = request.map(|request| CefString::from(&request.url()).to_string());
            if let Some(key) = url.as_deref().and_then(crate::scheme::chat_shown)
                && is_main
            {
                let _ = self.events.send(Event::ShowChat(key));
                // 1: do not go there.
                return 1;
            }
            let linked = url.and_then(|url| {
                crate::scheme::opened(&url)
                    .or_else(|| crate::links::file_linked(&url, user_gesture == 1))
            });
            match linked {
                Some(url) if is_main => {
                    let _ = self.events.send(Event::FileLink(url));
                    // 1: do not go there.
                    1
                }
                _ => 0,
            }
        }

        /// A request of the page's at a `cox:` address is answered by what
        /// the stage serves (see [`crate::scheme`]); the rest go to the
        /// network, or the file, as usual.
        fn resource_request_handler(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>,
            _is_navigation: c_int,
            _is_download: c_int,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut c_int>,
        ) -> Option<ResourceRequestHandler> {
            let serve = self.serve.as_ref()?;
            let url = CefString::from(&request?.url()).to_string();
            crate::scheme::is_served(&url).then(|| crate::scheme::handler(Arc::clone(serve)))
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
                let mut shared = lock(&self.shared);
                shared.navigation.loading = is_loading == 1;
                shared.navigation.can_go_back = can_go_back == 1;
                shared.navigation.can_go_forward = can_go_forward == 1;
            }
            let _ = self.events.send(Event::Navigated);
        }

        /// Another page started to load in the browser: not a frame of the
        /// page, nor the page moving within itself, as to an anchor.
        fn on_load_start(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: cef::TransitionType,
        ) {
            if frame.is_some_and(|frame| frame.is_main() == 1) {
                lock(&self.shared).favicons.loading();
            }
        }

        /// The page loaded: if it named no icon, it has none.
        fn on_load_end(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _http_status_code: c_int,
        ) {
            if frame.is_some_and(|frame| frame.is_main() == 1) {
                let next = lock(&self.shared).favicons.loaded();
                favicon::follow(&self.shared, &self.events, next);
            }
        }
    }
}

/// What CEF hands `on_cursor_change` as the platform's own cursor: an
/// `NSCursor` on macOS, which the cef crate names no type for, and an X
/// cursor elsewhere. The stage draws its cursor itself, so it goes unread.
#[cfg(target_os = "macos")]
type CursorHandle = *mut u8;
#[cfg(not(target_os = "macos"))]
type CursorHandle = cef::CursorHandle;

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
                lock(&self.shared).navigation.url = url.to_string();
                let _ = self.events.send(Event::Navigated);
            }
        }

        /// The page asks for another cursor where the mouse is. The stage
        /// shows it, as the browser has no window to show it in: so the
        /// change is taken here, and CEF told so.
        fn on_cursor_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            _cursor: CursorHandle,
            kind: cef::CursorType,
            _custom: Option<&cef::CursorInfo>,
        ) -> c_int {
            let cursor = Cursor::from(kind);
            let changed = std::mem::replace(&mut lock(&self.shared).cursor, cursor) != cursor;
            if changed {
                tracing::debug!(?cursor, "the page asks for a cursor");
                let _ = self.events.send(Event::Cursor);
            }
            1
        }

        /// The page's title changed, which its tab shows.
        fn on_title_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            title: Option<&CefString>,
        ) {
            lock(&self.shared).navigation.title =
                title.map(ToString::to_string).unwrap_or_default();
            let _ = self.events.send(Event::Navigated);
        }

        /// The page names its icons, or CEF the site's `/favicon.ico`
        /// for a page that names none: its tab shows the first that loads.
        fn on_favicon_urlchange(
            &self,
            _browser: Option<&mut cef::Browser>,
            icon_urls: Option<&mut cef::CefStringList>,
        ) {
            let urls = icon_urls
                .map(|urls| {
                    let urls: *mut cef::sys::_cef_string_list_t = urls.into();
                    // Borrowed from CEF, which frees it.
                    cef::CefStringList::from(urls).into_iter().collect()
                })
                .unwrap_or_default();
            tracing::debug!(?urls, "the page names its icons");
            let next = lock(&self.shared).favicons.named(urls);
            favicon::follow(&self.shared, &self.events, next);
        }
    }
}

wrap_life_span_handler! {
    struct StageLifeSpanHandler {
        shared: SharedRef,
        events: UnboundedSender<Event>,
        target: Arc<watch::Sender<Option<String>>>,
    }

    impl LifeSpanHandler {
        /// The page wants a window of its own, for a link with
        /// `target="_blank"` or `window.open`. The stage opens a tab
        /// for it instead.
        fn on_before_popup(
            &self,
            _browser: Option<&mut cef::Browser>,
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
            let url = target_url.map(ToString::to_string).unwrap_or_default();
            // `window.open()` with no address opens a blank page for
            // the script to fill, which a tab of the stage cannot be.
            if !url.is_empty() {
                let _ = self.events.send(Event::Popup(url));
            }
            // 1: no popup.
            1
        }

        /// The browser exists: it takes input now, and `DevTools` is
        /// asked for the page's target, which the page's agent is
        /// kept to. Unless the stage closed it already: then it closes.
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            if let Some(browser) = browser {
                let host = browser.host();
                {
                    let mut shared = lock(&self.shared);
                    shared.host.clone_from(&host);
                    if shared.closed {
                        drop(shared);
                        // CEF crashes if the browser is closed before this
                        // returns, so the close waits for a task.
                        if let Some(host) = host {
                            let mut task = CloseBrowser::new(host);
                            cef::post_task(
                                ThreadId::from(cef_thread_id_t::TID_UI),
                                Some(&mut task),
                            );
                        }
                        return;
                    }
                    // CEF lays the page out again at the size reported
                    // from here on, which may have changed since CEF
                    // made the page: the stage's next size waits for a
                    // picture of it, as after any resize.
                    shared.resizing = Some(Resizing::new(Instant::now()));
                }
                let mut devtools = None;
                if let Some(host) = &host {
                    // The scale too: on macOS CEF asks for it here, on
                    // this thread, which locks `shared`, so the lock is
                    // gone by then.
                    host.notify_screen_info_changed();
                    host.was_resized();
                    host.set_focus(1);
                    let mut observer = TargetObserver::new(Arc::clone(&self.target));
                    devtools = host.add_dev_tools_message_observer(Some(&mut observer));
                    host.execute_dev_tools_method(
                        0,
                        Some(&CefString::from("Target.getTargetInfo")),
                        None,
                    );
                }
                let mut shared = lock(&self.shared);
                shared.devtools = devtools;
                drop(shared);
                let _ = self.events.send(Event::Created);
            }
        }

        fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
            let mut shared = lock(&self.shared);
            shared.host = None;
            shared.devtools = None;
            drop(shared);
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
            let shared = lock(&self.shared);
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
            let shared = lock(&self.shared);
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
            let mut shared = lock(&self.shared);
            match *type_.as_ref() {
                cef_paint_element_type_t::PET_VIEW => {
                    shared.painted = Some((width, height));
                    let now = Instant::now();
                    shared.timeline.painted(now);
                    // The page changed: the window asks for its pictures
                    // at each frame for a while (see `frames`).
                    shared.pacing.stirred(now);
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
                let mut shared = lock(&self.shared);
                shared.popup = None;
                shared.popup_frame = None;
                drop(shared);
                let _ = self.events.send(Event::Painted);
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut cef::Browser>, rect: Option<&cef::Rect>) {
            if let Some(rect) = rect {
                lock(&self.shared).popup =
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
            lock(&self.shared).caret = caret;
        }
    }
}
