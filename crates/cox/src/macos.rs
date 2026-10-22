//! What CEF expects of the app on macOS and GPUI does not give it.
//!
//! Chromium asks `NSApp` whether it is inside `sendEvent:` (the
//! `CefAppProtocol` of CEF's examples) when it runs a loop of its own, as
//! for a context menu or a drag. GPUI's application class has no such
//! method, and an Objective-C object that gets a message it does not know
//! ends the process. So the methods are added to that class at run time,
//! with `sendEvent:` wrapped to keep the flag.

use std::ffi::CString;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::encode::{Encode, Encoding};
use objc2::ffi::{class_addMethod, class_addProtocol, object_getClass};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, Imp, Sel};
use objc2::{class, msg_send, sel};

/// Whether `NSApp` is inside `sendEvent:` right now. Events are sent on
/// the main thread only, but the flag is read from CEF's threads too.
static HANDLING_SEND_EVENT: AtomicBool = AtomicBool::new(false);

/// Teaches the application's class the protocol. Call it once, after GPUI
/// made the application and CEF's framework is loaded, and before the
/// browser is created.
///
/// # Panics
///
/// When the application has no class, which cannot happen.
pub fn adopt_cef_app_protocol() {
    // SAFETY: the shared application exists while GPUI runs, and the
    // methods added match the selectors' signatures.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let class = object_getClass(app).cast_mut();
        assert!(!class.is_null(), "the application has a class");
        add_method(
            class,
            sel!(isHandlingSendEvent),
            is_handling_send_event as extern "C-unwind" fn(_, _) -> _,
            &Bool::ENCODING,
            &[],
        );
        add_method(
            class,
            sel!(setHandlingSendEvent:),
            set_handling_send_event as extern "C-unwind" fn(_, _, _),
            &Encoding::Void,
            &[Bool::ENCODING],
        );
        add_method(
            class,
            sel!(sendEvent:),
            send_event as extern "C-unwind" fn(_, _, _),
            &Encoding::Void,
            &[<*mut AnyObject>::ENCODING],
        );
        // The protocol itself is declared by CEF's framework. Chromium only
        // checks for it in debug builds, so a missing one is not an error.
        if let Some(protocol) = AnyProtocol::get(c"CefAppProtocol") {
            class_addProtocol(class, protocol);
        }
    }
}

/// Adds a method to `class`, unless it already has one of that name.
///
/// # Safety
///
/// `imp` must be a function of `ret` and `args`, taking the receiver and
/// the selector first, and `class` must be a live class.
unsafe fn add_method<F: Copy>(
    class: *mut AnyClass,
    name: Sel,
    imp: F,
    ret: &Encoding,
    args: &[Encoding],
) {
    assert_eq!(
        size_of::<F>(),
        size_of::<Imp>(),
        "the implementation is a function pointer"
    );
    // The first two arguments of every method are the receiver and the
    // selector.
    let mut types = format!("{ret}{}{}", <*mut AnyObject>::ENCODING, Sel::ENCODING);
    for arg in args {
        write!(&mut types, "{arg}").expect("writing to a String");
    }
    let types = CString::new(types).expect("encodings have no NUL");
    // SAFETY: `F` is a function pointer of the right signature, as the
    // caller promised, and the same size as `Imp`.
    let imp: Imp = unsafe { std::mem::transmute_copy(&imp) };
    // SAFETY: `class` is live and `types` outlives the call.
    let added = unsafe { class_addMethod(class, name, imp, types.as_ptr()) };
    if !added.as_bool() {
        log::warn!("the application already answers {name}");
    }
}

extern "C-unwind" fn is_handling_send_event(_this: &AnyObject, _cmd: Sel) -> Bool {
    Bool::new(HANDLING_SEND_EVENT.load(Ordering::SeqCst))
}

extern "C-unwind" fn set_handling_send_event(_this: &AnyObject, _cmd: Sel, handling: Bool) {
    HANDLING_SEND_EVENT.store(handling.as_bool(), Ordering::SeqCst);
}

/// `NSApplication`'s `sendEvent:`, with the flag up while it runs. Events
/// nest (a menu's loop sends more), so the flag goes back to what it was.
extern "C-unwind" fn send_event(this: &AnyObject, _cmd: Sel, event: *mut AnyObject) {
    let was = HANDLING_SEND_EVENT.swap(true, Ordering::SeqCst);
    // SAFETY: `this` is the application, whose superclass is
    // `NSApplication`, and `event` is the event AppKit passed in.
    unsafe {
        let _: () = msg_send![super(this, class!(NSApplication)), sendEvent: event];
    }
    HANDLING_SEND_EVENT.store(was, Ordering::SeqCst);
}
