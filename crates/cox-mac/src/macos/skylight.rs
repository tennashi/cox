//! What Cox asks of `SkyLight`, the private framework under the window
//! server, that no public function does: where in its window an event
//! falls, and an app taken for the active one without its windows coming
//! to the front, so that it takes a click while it is behind another, as
//! yabai and Cua's driver have it. macOS may change or drop any of it, so
//! each function is looked up as it is first needed, and Cox goes on
//! without what it cannot find.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::LazyLock;

use objc2_core_foundation::CGPoint;
use objc2_core_graphics::CGEvent;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `SkyLight`, as `dlopen` hands it over, loaded for good: an address,
/// which a thread may pass to another.
static SKYLIGHT: LazyLock<Option<usize>> = LazyLock::new(|| {
    const RTLD_NOW: c_int = 0x2;
    let path = c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
    // SAFETY: a framework of the system, by a path that ends in NUL.
    let skylight = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
    if skylight.is_null() {
        tracing::debug!("cannot load SkyLight");
        return None;
    }
    Some(skylight as usize)
});

/// The function `name` of `SkyLight`, as `T`, the type it has.
///
/// # Safety
///
/// `T` is a function pointer of the function's very signature.
unsafe fn find<T: Copy>(name: &CStr) -> Option<T> {
    let skylight = (*SKYLIGHT)? as *mut c_void;
    // SAFETY: a name that ends in NUL, of a library that stays loaded.
    let found = unsafe { dlsym(skylight, name.as_ptr()) };
    if found.is_null() {
        tracing::debug!("SkyLight has no {name:?}");
        return None;
    }
    // SAFETY: as the caller says, and a function pointer is as wide as a
    // pointer.
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&found) })
}

/// `CGEventSetWindowLocation`, which sets where in its window an event is.
type SetWindowLocation = unsafe extern "C" fn(&CGEvent, CGPoint);

static SET_WINDOW_LOCATION: LazyLock<Option<SetWindowLocation>> =
    // SAFETY: the type is the function's.
    LazyLock::new(|| unsafe { find(c"CGEventSetWindowLocation") });

/// Sets where in its window `event` falls, `within` its top left corner,
/// in points: the mouse's events reach a window not under the mouse only
/// knowing it, as macOS 27 has it. Says whether it could.
pub(super) fn set_window_location(event: &CGEvent, within: CGPoint) -> bool {
    let Some(set_window_location) = *SET_WINDOW_LOCATION else {
        return false;
    };
    // SAFETY: an event Cox made, which lives through the call.
    unsafe { set_window_location(event, within) };
    true
}

/// A process as the window server numbers it, Carbon's
/// `ProcessSerialNumber`: two 32-bit halves.
type Psn = [u32; 2];

/// `CGSMainConnectionID`: Cox's connection to the window server.
type MainConnection = unsafe extern "C" fn() -> u32;
/// `SLSGetWindowOwner`: the connection of the app a window is of.
type WindowOwner = unsafe extern "C" fn(u32, u32, *mut u32) -> i32;
/// `SLSGetConnectionPSN`: the process of a connection.
type ConnectionPsn = unsafe extern "C" fn(u32, *mut Psn) -> i32;
/// `SLPSPostEventRecordTo`: posts one of the window server's own records
/// to a process.
type PostEventRecord = unsafe extern "C" fn(*const Psn, *const u8) -> i32;

/// What [`focus`] needs of `SkyLight`.
struct Focusing {
    main_connection: MainConnection,
    window_owner: WindowOwner,
    connection_psn: ConnectionPsn,
    post_event_record: PostEventRecord,
}

static FOCUSING: LazyLock<Option<Focusing>> = LazyLock::new(|| {
    // SAFETY: each type is its function's, as yabai and Cua's driver call
    // them.
    unsafe {
        Some(Focusing {
            main_connection: find(c"CGSMainConnectionID")?,
            window_owner: find(c"SLSGetWindowOwner")?,
            connection_psn: find(c"SLSGetConnectionPSN")?,
            post_event_record: find(c"SLPSPostEventRecordTo")?,
        })
    }
});

/// How long a record is, in bytes.
const RECORD: usize = 0xf8;

/// The record that tells the app of `window` its window is the active
/// one, or no longer is: the window server's own, as yabai found it.
fn record(window: u32, focused: bool) -> [u8; RECORD] {
    let mut record = [0; RECORD];
    record[0x04] = 0xf8;
    record[0x08] = 0x0d;
    record[0x3c..0x40].copy_from_slice(&window.to_le_bytes());
    record[0x8a] = if focused { 0x01 } else { 0x02 };
    record
}

/// Has the app of `window` take itself for the active app, with `window`
/// its active window, or no longer, while its windows stay where they
/// are and the app in front stays in front, with the keyboard: then it
/// takes a click while it is behind another, as Chrome's pages do, where
/// it would let one go. Says whether the window server took the record.
pub(super) fn focus(window: u32, focused: bool) -> bool {
    let Some(focusing) = &*FOCUSING else {
        return false;
    };
    let mut owner = 0;
    let mut psn: Psn = [0; 2];
    // SAFETY: functions of the signatures they were found as, given what
    // they write into, which lives through the calls.
    let posted = unsafe {
        let connection = (focusing.main_connection)();
        (focusing.window_owner)(connection, window, &raw mut owner) == 0
            && owner != 0
            && (focusing.connection_psn)(owner, &raw mut psn) == 0
            && (focusing.post_event_record)(&raw const psn, record(window, focused).as_ptr()) == 0
    };
    if posted {
        tracing::debug!(window, focused, "posted a focus record");
    } else {
        tracing::debug!(window, focused, "the window server took no focus record");
    }
    posted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_names_its_window_and_which_way() {
        let focus = record(0x0102_0304, true);
        assert_eq!(focus.len(), 248);
        assert_eq!(&focus[0x3c..0x40], &[4, 3, 2, 1]);
        assert_eq!(focus[0x8a], 1);
        assert_eq!(record(7, false)[0x8a], 2);
    }
}
