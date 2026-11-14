//! Quitting from places that cannot quit right away.
//!
//! Quitting runs the app's quit handlers, which shut CEF down and stop the
//! agent. A signal handler can do none of that, and neither can CEF's turn
//! on the main thread on macOS (see `cox_browser::macos`). Those only
//! [`ask`], and the app quits when it next looks ([`take_asked`]), as coxd
//! does.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether something asked Cox to quit since the app last looked.
static ASKED: AtomicBool = AtomicBool::new(false);

/// Asks Cox to quit. Safe to call from a signal handler.
pub fn ask() {
    ASKED.store(true, Ordering::SeqCst);
}

/// Whether Cox was asked to quit since the last call.
pub fn take_asked() -> bool {
    ASKED.swap(false, Ordering::SeqCst)
}

/// Makes SIGTERM, SIGINT and SIGHUP ask Cox to quit. Call it after CEF is
/// up: CEF handles them too, by quitting its own way, which skips Cox's
/// quit handlers and leaves the agent running, or on macOS crashes.
///
/// # Errors
///
/// When a handler cannot be installed.
pub fn on_signals() -> io::Result<()> {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: the handler only stores to an atomic, which is safe in a
        // signal handler, and the action is fully set up before it is
        // installed in place of CEF's.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&raw mut action.sa_mask);
            if libc::sigaction(signal, &raw const action, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

extern "C" fn on_signal(_signal: libc::c_int) {
    ask();
}
