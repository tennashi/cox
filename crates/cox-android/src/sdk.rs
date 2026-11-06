//! The Android SDK's emulator: where its program is.

use std::path::PathBuf;

/// The emulator's program: `emulator/emulator` in the SDK that
/// `ANDROID_HOME` names, or `ANDROID_SDK_ROOT`, or else where Android
/// Studio puts the SDK, if it is there.
#[must_use]
pub fn emulator() -> Option<PathBuf> {
    sdk_roots()
        .into_iter()
        .map(|root| root.join("emulator").join("emulator"))
        .find(|program| program.is_file())
}

fn sdk_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Android/sdk"));
        } else {
            roots.push(home.join("Android/Sdk"));
        }
    }
    roots
}
