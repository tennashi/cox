//! The virtual devices (AVDs) the emulator runs, as Android Studio's
//! Device Manager keeps them: each is `<name>.ini` in the AVDs' folder,
//! which names the folder of the device, `<name>.avd`, whose `config.ini`
//! says what it is. Cox lists them and says what each is; making one,
//! changing it, deleting it, and the system images they run, are left to
//! Android Studio.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A virtual device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avd {
    /// Its name, as `emulator -avd` takes it.
    pub name: String,
    /// The name the human gave it, or its name.
    pub title: String,
    /// The phone it is made as, as `pixel_9`, if it says.
    pub model: Option<String>,
    /// The Android API level it runs, as `35`, if it says.
    pub api: Option<String>,
    /// The size of its display, in its own pixels, if it says.
    pub display: Option<(u32, u32)>,
    /// Its folder.
    pub folder: PathBuf,
}

/// The AVDs' folder, as the SDK's tools find it: `ANDROID_AVD_HOME`, or
/// else `avd` in `ANDROID_EMULATOR_HOME`, or in `ANDROID_USER_HOME`, or
/// else `~/.android/avd`.
#[must_use]
pub fn home() -> Option<PathBuf> {
    home_from(|name| std::env::var_os(name))
}

/// The AVDs' folder, as [`home`] finds it, with `env` saying the
/// environment's variables.
#[must_use]
pub fn home_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(home) = set("ANDROID_AVD_HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(user) = set("ANDROID_EMULATOR_HOME").or_else(|| set("ANDROID_USER_HOME")) {
        return Some(PathBuf::from(user).join("avd"));
    }
    set("HOME").map(|home| PathBuf::from(home).join(".android/avd"))
}

/// The virtual devices in the AVDs' folder, by name.
#[must_use]
pub fn all() -> Vec<Avd> {
    home().map(|home| in_folder(&home)).unwrap_or_default()
}

/// The virtual devices in `home`, by name.
#[must_use]
pub fn in_folder(home: &Path) -> Vec<Avd> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut avds: Vec<Avd> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.strip_suffix(".ini")?.to_owned();
            let ini = std::fs::read_to_string(&path).ok()?;
            Some(read(name, home, &ini))
        })
        .collect();
    avds.sort_by(|a, b| a.name.cmp(&b.name));
    avds
}

/// The device `name`, whose `<name>.ini` in `home` says `ini`.
fn read(name: String, home: &Path, ini: &str) -> Avd {
    let folder = value(ini, "path")
        .map(PathBuf::from)
        .filter(|folder| folder.is_dir())
        .or_else(|| value(ini, "path.rel").map(|relative| home.join("..").join(relative)))
        .unwrap_or_else(|| home.join(format!("{name}.avd")));
    let config = std::fs::read_to_string(folder.join("config.ini")).unwrap_or_default();
    let api = value(ini, "target")
        .or_else(|| value(&config, "image.sysdir.1"))
        .and_then(api_of);
    let display = value(&config, "hw.lcd.width")
        .zip(value(&config, "hw.lcd.height"))
        .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)));
    Avd {
        title: value(&config, "avd.ini.displayname").map_or_else(|| name.clone(), str::to_owned),
        model: value(&config, "hw.device.name").map(str::to_owned),
        api,
        display,
        folder,
        name,
    }
}

/// The value of `key` in `ini`, a `key=value` a line.
fn value<'a>(ini: &'a str, key: &str) -> Option<&'a str> {
    ini.lines()
        .find_map(|line| {
            let (found, value) = line.split_once('=')?;
            (found.trim() == key).then(|| value.trim())
        })
        .filter(|value| !value.is_empty())
}

/// The API level in `android-35`, or a path with it, as
/// `system-images/android-35/google_apis/arm64-v8a/`.
fn api_of(text: &str) -> Option<String> {
    text.split(['/', '\\'])
        .find_map(|part| part.strip_prefix("android-"))
        .map(str::to_owned)
}

/// Writes a device `name` in `home`, as Android Studio would, with
/// `config`, for `cox-fake-emulator` and the tests to have one.
///
/// # Errors
///
/// When its files cannot be written.
pub fn write(home: &Path, name: &str, config: &str) -> std::io::Result<PathBuf> {
    let folder = home.join(format!("{name}.avd"));
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("config.ini"), config)?;
    std::fs::write(
        home.join(format!("{name}.ini")),
        format!(
            "avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/{name}.avd\n",
            folder.display()
        ),
    )?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env;
    use crate::scratch::Scratch;

    #[test]
    fn says_what_each_device_is() {
        let scratch = Scratch::new("avd-says");
        let home = scratch.path();
        let folder = write(
            home,
            "oes-handy",
            "avd.ini.displayname=OES Handy\nhw.device.name=pixel_9\n\
             image.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n\
             hw.lcd.width=1080\nhw.lcd.height=2424\n",
        )
        .unwrap();
        assert_eq!(
            in_folder(home),
            [Avd {
                name: "oes-handy".into(),
                title: "OES Handy".into(),
                model: Some("pixel_9".into()),
                api: Some("35".into()),
                display: Some((1080, 2424)),
                folder,
            }]
        );
    }

    #[test]
    fn a_device_that_says_nothing_is_titled_by_its_name() {
        let scratch = Scratch::new("avd-bare");
        let folder = write(scratch.path(), "bare", "").unwrap();
        assert_eq!(
            in_folder(scratch.path()),
            [Avd {
                name: "bare".into(),
                title: "bare".into(),
                model: None,
                api: None,
                display: None,
                folder,
            }]
        );
    }

    #[test]
    fn devices_come_by_name() {
        let scratch = Scratch::new("avd-by-name");
        for name in ["zeta", "alpha", "mid"] {
            write(scratch.path(), name, "").unwrap();
        }
        let names: Vec<String> = in_folder(scratch.path())
            .into_iter()
            .map(|avd| avd.name)
            .collect();
        assert_eq!(names, ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn a_folder_that_is_not_there_has_no_devices() {
        let scratch = Scratch::new("avd-missing");
        assert_eq!(in_folder(&scratch.path().join("missing")), []);
    }

    #[test]
    fn the_api_level_of_the_target_comes_before_the_images() {
        let scratch = Scratch::new("avd-target");
        let home = scratch.path();
        let folder = write(
            home,
            "targeted",
            "image.sysdir.1=system-images/android-34/google_apis/arm64-v8a/\n",
        )
        .unwrap();
        std::fs::write(
            home.join("targeted.ini"),
            format!("path={}\ntarget=android-36\n", folder.display()),
        )
        .unwrap();
        assert_eq!(in_folder(home)[0].api.as_deref(), Some("36"));
    }

    #[test]
    fn the_api_level_is_read_from_the_image_with_its_minor_version() {
        let scratch = Scratch::new("avd-image");
        write(
            scratch.path(),
            "minor",
            "image.sysdir.1=system-images/android-36.1/google_apis_playstore/arm64-v8a/\n",
        )
        .unwrap();
        assert_eq!(in_folder(scratch.path())[0].api.as_deref(), Some("36.1"));
    }

    #[test]
    fn an_image_with_no_api_level_says_none() {
        let scratch = Scratch::new("avd-no-api");
        write(scratch.path(), "odd", "image.sysdir.1=google_apis/\n").unwrap();
        assert_eq!(in_folder(scratch.path())[0].api, None);
    }

    #[test]
    fn a_device_whose_path_is_gone_is_found_by_its_relative_path() {
        let scratch = Scratch::new("avd-relative");
        let home = scratch.path().join("avd");
        write(&home, "moved", "hw.lcd.width=1\nhw.lcd.height=2\n").unwrap();
        std::fs::write(
            home.join("moved.ini"),
            "path=/gone/moved.avd\npath.rel=avd/moved.avd\n",
        )
        .unwrap();
        let found = &in_folder(&home)[0];
        assert_eq!(found.folder, home.join("..").join("avd/moved.avd"));
        assert_eq!(found.display, Some((1, 2)), "its config is read there");
    }

    #[test]
    fn android_avd_home_comes_first() {
        let found = home_from(env(&[
            ("ANDROID_AVD_HOME", Path::new("/avds")),
            ("ANDROID_EMULATOR_HOME", Path::new("/emulator")),
            ("ANDROID_USER_HOME", Path::new("/user")),
            ("HOME", Path::new("/home")),
        ]));
        assert_eq!(found, Some(PathBuf::from("/avds")));
    }

    #[test]
    fn android_emulator_home_comes_before_android_user_home() {
        let found = home_from(env(&[
            ("ANDROID_AVD_HOME", Path::new("")),
            ("ANDROID_EMULATOR_HOME", Path::new("/emulator")),
            ("ANDROID_USER_HOME", Path::new("/user")),
            ("HOME", Path::new("/home")),
        ]));
        assert_eq!(found, Some(PathBuf::from("/emulator/avd")));
    }

    #[test]
    fn android_user_home_comes_before_home() {
        let found = home_from(env(&[
            ("ANDROID_USER_HOME", Path::new("/user")),
            ("HOME", Path::new("/home")),
        ]));
        assert_eq!(found, Some(PathBuf::from("/user/avd")));
    }

    #[test]
    fn with_no_android_variable_the_devices_are_in_home() {
        let found = home_from(env(&[("HOME", Path::new("/home"))]));
        assert_eq!(found, Some(PathBuf::from("/home/.android/avd")));
        assert_eq!(home_from(env(&[])), None);
    }
}
