//! coxd started as the human logs in, so that the chats' schedules run and
//! the phone reaches Cox before the human opens its window: the launch agent
//! Cox.app carries, in `Contents/Library/LaunchAgents`, which an installed
//! Cox hands the system the first time it opens. The human sees it in
//! System Settings, under General, Login Items, and can turn it off there;
//! Cox leaves it off then.

use objc2_foundation::NSString;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// The launch agent's file in the app, named as launchd knows coxd.
const PLIST: &str = "com.github.tennashi.cox.coxd.plist";

/// Hands the system the launch agent that starts coxd at login, once: for a
/// Cox that is installed (see [`coxd::storage::installed`]), and not again
/// once the human has turned it off.
pub fn keep() {
    if !coxd::storage::installed() {
        return;
    }
    // SAFETY: the plist's name is a string, and the service is asked only
    // how it stands and to register.
    let service = unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(PLIST)) };
    match unsafe { service.status() } {
        SMAppServiceStatus::NotRegistered => match unsafe { service.registerAndReturnError() } {
            Ok(()) => tracing::info!("coxd starts at login from now on"),
            Err(error) => tracing::warn!(
                "coxd does not start at login: the system did not take its launch agent: {}",
                error.localizedDescription()
            ),
        },
        SMAppServiceStatus::RequiresApproval => {
            tracing::info!("coxd does not start at login: it is off in Login Items");
        }
        SMAppServiceStatus::NotFound => {
            tracing::warn!("coxd does not start at login: the app carries no {PLIST}");
        }
        _ => {}
    }
}
