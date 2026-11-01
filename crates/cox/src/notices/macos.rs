//! The system's notifications, and the Dock's badge, on macOS.
//!
//! The notifications go through `NSUserNotificationCenter`, which macOS
//! has deprecated for `UNUserNotificationCenter`: that one asks the human
//! to allow an app's notifications only when the app is signed with a
//! certificate, and refuses Cox, which `cargo xtask bundle` signs ad hoc.

#![expect(
    deprecated,
    reason = "UNUserNotificationCenter refuses an app signed ad hoc"
)]

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::NSApplication;
use objc2_foundation::{
    NSString, NSUserNotification, NSUserNotificationCenter, NSUserNotificationCenterDelegate,
};
use tokio::sync::mpsc::UnboundedSender;

use super::Notices;

/// What the identifier of a chat's notification starts with, before the
/// chat's number.
const PREFIX: &str = "cox-chat-";

/// The system's notifications, which a click on one tells of, and the
/// Dock's badge.
pub struct System {
    center: Retained<NSUserNotificationCenter>,
    /// Kept: the center holds it weakly.
    delegate: Retained<Delegate>,
    main_thread: MainThreadMarker,
}

impl System {
    /// Sends the number of the chat of each notification the human clicks
    /// to `clicked`.
    ///
    /// # Panics
    ///
    /// Off the main thread.
    #[must_use]
    pub fn start(clicked: UnboundedSender<u64>) -> Self {
        let main_thread = MainThreadMarker::new().expect("Cox starts on the main thread");
        let center = NSUserNotificationCenter::defaultUserNotificationCenter();
        let system = Self {
            center,
            delegate: Delegate::new(clicked),
            main_thread,
        };
        system.claim();
        system
    }

    /// Makes the delegate the center's, again if something else in Cox,
    /// as CEF, has made itself the center's delegate since.
    fn claim(&self) {
        // SAFETY: the center returns its delegate retained.
        let current = unsafe { self.center.delegate() };
        let ours = ProtocolObject::from_ref(&*self.delegate);
        if current
            .as_deref()
            .is_some_and(|current| std::ptr::eq(current, ours))
        {
            return;
        }
        if current.is_some() {
            tracing::warn!("something else took the notifications' delegate; Cox takes it back");
        }
        // SAFETY: `System` keeps the delegate as long as the center.
        unsafe { self.center.setDelegate(Some(ours)) };
    }
}

impl Notices for System {
    fn notify(&self, chat: u64, title: &str, body: &str) {
        let notification = NSUserNotification::new();
        notification.setTitle(Some(&NSString::from_str(title)));
        notification.setInformativeText(Some(&NSString::from_str(body)));
        notification.setSoundName(Some(&NSString::from_str(
            "NSUserNotificationDefaultSoundName",
        )));
        // The same identifier replaces what was told of the chat before.
        notification.setIdentifier(Some(&NSString::from_str(&format!("{PREFIX}{chat}"))));
        // Without the delegate, a notification does not show while Cox is
        // in front, and a click on it does nothing.
        self.claim();
        self.center.deliverNotification(&notification);
    }

    fn badge(&self, count: usize) {
        let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
        NSApplication::sharedApplication(self.main_thread)
            .dockTile()
            .setBadgeLabel(label.as_deref());
    }
}

struct Ivars {
    clicked: UnboundedSender<u64>,
}

define_class!(
    /// Hears the human click a notification, and lets notifications show
    /// while Cox is in front: Cox tells only of chats the human is not
    /// looking at.
    #[unsafe(super(NSObject))]
    #[name = "CoxNotificationsDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:shouldPresentNotification:))]
        fn should_present(
            &self,
            _center: &NSUserNotificationCenter,
            _notification: &NSUserNotification,
        ) -> bool {
            true
        }

        #[unsafe(method(userNotificationCenter:didActivateNotification:))]
        fn did_activate(
            &self,
            center: &NSUserNotificationCenter,
            notification: &NSUserNotification,
        ) {
            let chat = notification
                .identifier()
                .and_then(|identifier| identifier.to_string().strip_prefix(PREFIX)?.parse().ok());
            if let Some(chat) = chat {
                let _ = self.ivars().clicked.send(chat);
            }
            center.removeDeliveredNotification(notification);
        }
    }
);

impl Delegate {
    fn new(clicked: UnboundedSender<u64>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { clicked });
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}
