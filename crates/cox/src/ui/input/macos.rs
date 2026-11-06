//! Keys with Control for the input method, on macOS.
//!
//! macOS hands a key with Control to the window as a key equivalent first,
//! and GPUI, when no binding takes it, hands it to no one: not to the input
//! method, and not as the key down that follows, which GPUI takes for the
//! same key again and drops. Some of those keys are the input method's own,
//! as Control-J, which turns macSKK's kana on again from its ASCII mode. So
//! an input hands the input method a key with Control that nothing in Cox
//! took, as an `NSTextView` would.

use gpui::App;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSEventType, NSView};

/// Hands the key the system is handing out now to the input method of the
/// view that has the keyboard, once GPUI is done with the key: true when
/// there is such a key.
///
/// The input method answers through GPUI, which takes what it says only
/// while it is not handling an event, so the key goes to it after.
pub(super) fn hand_on_key(cx: &App) -> bool {
    let Some(main_thread) = MainThreadMarker::new() else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(main_thread).currentEvent() else {
        return false;
    };
    if event.r#type() != NSEventType::KeyDown {
        return false;
    }
    cx.foreground_executor()
        .spawn(async move { hand_to_input_method(&event, main_thread) })
        .detach();
    true
}

/// Hands `event` to the input method of the view that has the keyboard in
/// the event's window.
fn hand_to_input_method(event: &NSEvent, main_thread: MainThreadMarker) {
    let Some(view) = event
        .window(main_thread)
        .and_then(|window| window.firstResponder())
        .and_then(|responder| Retained::downcast::<NSView>(responder).ok())
    else {
        return;
    };
    if let Some(context) = view.inputContext() {
        context.handleEvent(event);
    }
}
