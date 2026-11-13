//! The Android SDK's emulator: where its program is.

use std::ffi::OsString;
use std::path::PathBuf;

/// The emulator's program: `emulator/emulator` in the SDK that
/// `ANDROID_HOME` names, or `ANDROID_SDK_ROOT`, or else where Android
/// Studio puts the SDK, if it is there.
#[must_use]
pub fn emulator() -> Option<PathBuf> {
    emulator_from(|name| std::env::var_os(name))
}

/// The emulator's program, as [`emulator`] finds it, with `env` saying
/// the environment's variables.
#[must_use]
pub fn emulator_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    sdk_roots(env)
        .into_iter()
        .map(|root| root.join("emulator").join("emulator"))
        .find(|program| program.is_file())
}

fn sdk_roots(env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .into_iter()
        .filter_map(&env)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .collect();
    if let Some(home) = env("HOME").map(PathBuf::from) {
        if cfg!(target_os = "macos") {
            roots.push(home.join("Library/Android/sdk"));
        } else {
            roots.push(home.join("Android/Sdk"));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::env;
    use crate::scratch::Scratch;

    /// An SDK in `root`, with an emulator's program, which it gives.
    fn sdk(root: &Path) -> PathBuf {
        let program = root.join("emulator/emulator");
        std::fs::create_dir_all(root.join("emulator")).unwrap();
        std::fs::write(&program, "").unwrap();
        program
    }

    /// Where Android Studio puts the SDK, in `home`.
    fn studio_sdk(home: &Path) -> PathBuf {
        if cfg!(target_os = "macos") {
            home.join("Library/Android/sdk")
        } else {
            home.join("Android/Sdk")
        }
    }

    #[test]
    fn android_home_comes_before_android_sdk_root_and_android_studio() {
        let scratch = Scratch::new("sdk-android-home");
        let (named, root, home) = (
            scratch.path().join("named"),
            scratch.path().join("root"),
            scratch.path().join("home"),
        );
        let program = sdk(&named);
        sdk(&root);
        sdk(&studio_sdk(&home));
        let found = emulator_from(env(&[
            ("ANDROID_HOME", &named),
            ("ANDROID_SDK_ROOT", &root),
            ("HOME", &home),
        ]));
        assert_eq!(found, Some(program));
    }

    #[test]
    fn android_sdk_root_comes_before_android_studio() {
        let scratch = Scratch::new("sdk-sdk-root");
        let (root, home) = (scratch.path().join("root"), scratch.path().join("home"));
        let program = sdk(&root);
        sdk(&studio_sdk(&home));
        let found = emulator_from(env(&[("ANDROID_SDK_ROOT", &root), ("HOME", &home)]));
        assert_eq!(found, Some(program));
    }

    #[test]
    fn an_sdk_with_no_emulator_or_an_empty_variable_is_passed_over() {
        let scratch = Scratch::new("sdk-passed-over");
        let (bare, home) = (scratch.path().join("bare"), scratch.path().join("home"));
        std::fs::create_dir_all(&bare).unwrap();
        let program = sdk(&studio_sdk(&home));
        let found = emulator_from(env(&[
            ("ANDROID_HOME", &bare),
            ("ANDROID_SDK_ROOT", Path::new("")),
            ("HOME", &home),
        ]));
        assert_eq!(found, Some(program));
    }

    #[test]
    fn no_sdk_has_no_emulator() {
        let scratch = Scratch::new("sdk-none");
        assert_eq!(emulator_from(env(&[("HOME", scratch.path())])), None);
    }
}
