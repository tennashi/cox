//! The windows of other apps on macOS: `ScreenCaptureKit` lists them and
//! streams a picture of one as it changes.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use anyhow::Context as _;
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use objc2_core_foundation::CGRect;
use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::NSError;
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{Event, Frame, Rect, Shared, Size, Window, fitting};

/// How long the system has to answer what Cox asks of it.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// How often the mirror looks again where its window is, as the human
/// moves it, and whether it is still open.
const LOOK_EVERY: Duration = Duration::from_millis(500);

/// How many pixels the pictures are, at most each way, until the stage
/// says how big it draws them.
const FIRST_SIZE: u32 = 2048;

/// The fewest points a window is wide and high to be listed: smaller ones
/// are a menu's or a status item's.
const SMALLEST: f64 = 40.;

/// What `ScreenCaptureKit` hands over from the queue it answers on.
struct Sendable<T>(T);

// SAFETY: ScreenCaptureKit's content, windows and errors are immutable
// once made, and Apple's samples pass them between queues.
unsafe impl<T> Send for Sendable<T> {}

/// What `error` says, if it is set.
fn said(error: *mut NSError) -> String {
    // SAFETY: the system passes an error or null.
    unsafe { error.as_ref() }.map_or_else(
        || "the system did not say why".to_owned(),
        |error| error.localizedDescription().to_string(),
    )
}

/// What the system shares to capture now: the windows on screen, but the
/// desktop's.
fn shareable() -> anyhow::Result<Retained<SCShareableContent>> {
    // Asked while Cox may not record the screen, the system shows its
    // dialog again each time, as the list of windows asks every few
    // seconds: only the human's "Allow…" asks.
    if !may_record() {
        anyhow::bail!("Cox may not record the screen yet");
    }
    let (sent, answer) = std::sync::mpsc::channel();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: the system passes its content, or null with an error.
            let content = unsafe { Retained::retain(content) };
            let _ = sent.send(Sendable(content.ok_or_else(|| said(error))));
        },
    );
    // SAFETY: the handler is retained by the system until it answers.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &handler,
        );
    }
    let Sendable(content) = answer
        .recv_timeout(ANSWER_WITHIN)
        .context("the system did not say which windows there are")?;
    content.map_err(|why| anyhow::anyhow!("the system did not say which windows there are: {why}"))
}

fn rect(frame: CGRect) -> Rect {
    Rect {
        x: frame.origin.x,
        y: frame.origin.y,
        width: frame.size.width,
        height: frame.size.height,
    }
}

/// `window` as Cox lists it, if it is another app's window on screen that
/// a human works in.
fn listed(window: &SCWindow, own: i32) -> Option<Window> {
    // SAFETY: plain getters of an immutable object.
    unsafe {
        let app = window.owningApplication()?;
        let pid = app.processID();
        let frame = rect(window.frame());
        let shown = window.isOnScreen()
            && window.windowLayer() == 0
            && frame.width >= SMALLEST
            && frame.height >= SMALLEST;
        (shown && pid != own).then(|| Window {
            id: window.windowID(),
            pid,
            app: app.applicationName().to_string(),
            title: window
                .title()
                .map(|title| title.to_string())
                .unwrap_or_default(),
            frame,
        })
    }
}

pub(crate) fn windows() -> anyhow::Result<Vec<Window>> {
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    let content = shareable()?;
    // SAFETY: a getter of an immutable object.
    let windows = unsafe { content.windows() };
    let mut listed: Vec<Window> = windows
        .iter()
        .filter_map(|window| listed(&window, own))
        .filter(|window| !window.app.is_empty())
        .collect();
    listed.sort_by(|a, b| (&a.app, &a.title, a.id).cmp(&(&b.app, &b.title, b.id)));
    Ok(listed)
}

/// The window `id`, as the system shares it now, and as Cox lists it.
fn find(id: u32) -> anyhow::Result<Option<(Retained<SCWindow>, Window)>> {
    let own = i32::try_from(std::process::id()).unwrap_or_default();
    let content = shareable()?;
    // SAFETY: a getter of an immutable object.
    let windows = unsafe { content.windows() };
    Ok(windows.iter().find_map(|window| {
        // SAFETY: a getter of an immutable object.
        if unsafe { window.windowID() } != id {
            return None;
        }
        let listed = listed(&window, own)?;
        Some((window, listed))
    }))
}

pub(crate) fn may_record() -> bool {
    CGPreflightScreenCaptureAccess()
}

pub(crate) fn ask_to_record() {
    CGRequestScreenCaptureAccess();
}

pub(crate) fn bring_to_front(pid: i32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    }
}

struct Ivars {
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
}

define_class!(
    /// Takes the pictures of a window as `ScreenCaptureKit` streams them,
    /// and hears the stream stop, as when the window closes.
    #[unsafe(super(NSObject))]
    #[name = "CoxWindowMirror"]
    #[ivars = Ivars]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(
            &self,
            _stream: &SCStream,
            buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind != SCStreamOutputType::Screen {
                return;
            }
            if let Some(frame) = picture(buffer) {
                let ivars = self.ivars();
                ivars.shared.put(frame, &ivars.events);
            }
        }
    }

    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _stream: &SCStream, error: &NSError) {
            let why = error.localizedDescription().to_string();
            let _ = self.ivars().events.send(Event::Ended(why));
        }
    }
);

impl Output {
    fn new(shared: Arc<Shared>, events: UnboundedSender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { shared, events });
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

/// The picture in `buffer`, rows packed, if it holds one: a buffer the
/// stream sends while the window does not change holds none.
fn picture(buffer: &CMSampleBuffer) -> Option<Frame> {
    // SAFETY: the stream hands over a valid buffer.
    let image = unsafe { buffer.image_buffer() }?;
    // SAFETY: the buffer is locked while its bytes are read, and they are
    // as many as its rows times their stride.
    unsafe {
        if CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) != 0 {
            return None;
        }
        let width = CVPixelBufferGetWidth(&image);
        let height = CVPixelBufferGetHeight(&image);
        let stride = CVPixelBufferGetBytesPerRow(&image);
        let base = CVPixelBufferGetBaseAddress(&image)
            .cast::<u8>()
            .cast_const();
        let frame = (!base.is_null() && stride >= width * 4).then(|| {
            let bytes = std::slice::from_raw_parts(base, stride * height);
            let mut bgra = Vec::with_capacity(width * height * 4);
            for row in bytes.chunks_exact(stride) {
                bgra.extend_from_slice(&row[..width * 4]);
            }
            Frame {
                width: u32::try_from(width).unwrap_or_default(),
                height: u32::try_from(height).unwrap_or_default(),
                bgra,
            }
        });
        CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
        frame
    }
}

/// What to ask the stream for: the window's pictures `size` big, as often
/// as the screen draws it.
fn configuration(size: (u32, u32)) -> Retained<SCStreamConfiguration> {
    // SAFETY: setters of a configuration Cox made.
    unsafe {
        let configuration = SCStreamConfiguration::new();
        configuration.setWidth(size.0 as usize);
        configuration.setHeight(size.1 as usize);
        configuration.setPixelFormat(kCVPixelFormatType_32BGRA);
        configuration.setMinimumFrameInterval(CMTime::new(1, 60));
        configuration.setShowsCursor(false);
        configuration.setQueueDepth(3);
        configuration.setIgnoreShadowsSingleWindow(true);
        // The window alone: with the windows it opens, as the menu of a
        // right click, which macOS opens on screen however far from the
        // window, the picture would shrink to make room for them. Before
        // macOS 14.2, which has no such setting, the windows come too.
        if configuration.respondsToSelector(sel!(setIncludeChildWindows:)) {
            configuration.setIncludeChildWindows(false);
        }
        configuration
    }
}

/// Waits for the system to call the handler `ask` gives it.
fn answered(ask: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>)) -> Result<(), String> {
    let (sent, answer) = std::sync::mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        let _ = sent.send((!error.is_null()).then(|| said(error)));
    });
    ask(&handler);
    match answer.recv_timeout(ANSWER_WITHIN) {
        Ok(None) => Ok(()),
        Ok(Some(why)) => Err(why),
        Err(_) => Err("the system did not answer".to_owned()),
    }
}

pub(crate) fn mirror(
    id: u32,
    shared: &Arc<Shared>,
    sizes: &Receiver<Size>,
    events: &UnboundedSender<Event>,
) {
    if let Err(why) = show(id, shared, sizes, events) {
        let _ = events.send(Event::Ended(why));
    }
}

fn show(
    id: u32,
    shared: &Arc<Shared>,
    sizes: &Receiver<Size>,
    events: &UnboundedSender<Event>,
) -> Result<(), String> {
    if !may_record() {
        return Err(
            "Cox may not record the screen yet: allow it in System Settings, \
                    Privacy & Security, Screen & System Audio Recording, then open Cox again."
                .to_owned(),
        );
    }
    let (found, mut window) = find(id)
        .map_err(|error| format!("{error:#}"))?
        .ok_or_else(|| "The window is closed.".to_owned())?;
    // Until the stage says how big it draws it.
    let mut asked = (FIRST_SIZE, FIRST_SIZE);
    let mut size = fitting(window.frame, asked.0, asked.1);
    let configuration = configuration(size);
    let output = Output::new(Arc::clone(shared), events.clone());
    // SAFETY: the filter, the configuration and the output live as long as
    // the stream, which this function holds until it stops it.
    let stream = unsafe {
        let filter =
            SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &found);
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &configuration,
            Some(ProtocolObject::from_ref(&*output)),
        )
    };
    let queue = DispatchQueue::new("cox-mac-window", None);
    // SAFETY: as above.
    unsafe {
        stream.addStreamOutput_type_sampleHandlerQueue_error(
            ProtocolObject::from_ref(&*output),
            SCStreamOutputType::Screen,
            Some(&queue),
        )
    }
    .map_err(|error| error.localizedDescription().to_string())?;
    // SAFETY: as above.
    answered(|handler| unsafe { stream.startCaptureWithCompletionHandler(Some(handler)) })?;
    let _ = events.send(Event::Showing);
    let ended = loop {
        match sizes.recv_timeout(LOOK_EVERY) {
            Ok(size) => asked = size,
            Err(RecvTimeoutError::Timeout) => match find(id) {
                Ok(Some((_, now))) => window = now,
                Ok(None) => break Some("The window closed.".to_owned()),
                Err(error) => tracing::debug!("cannot look at window {id}: {error:#}"),
            },
            Err(RecvTimeoutError::Disconnected) => break None,
        }
        let fits = fitting(window.frame, asked.0, asked.1);
        if fits != size {
            size = fits;
            // SAFETY: as above, and setters of a configuration Cox made.
            unsafe {
                configuration.setWidth(size.0 as usize);
                configuration.setHeight(size.1 as usize);
                stream.updateConfiguration_completionHandler(&configuration, None);
            }
        }
    };
    // SAFETY: as above.
    unsafe { stream.stopCaptureWithCompletionHandler(None) };
    ended.map_or(Ok(()), Err)
}
