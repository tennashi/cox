//! The virtual devices (AVDs) the emulator runs, as Android Studio's
//! Device Manager keeps them: each is `<name>.ini` in the AVDs' folder,
//! which names the folder of the device, `<name>.avd`, whose `config.ini`
//! says what it is. Cox lists them and says what each is; making one,
//! changing it, deleting it, and the system images they run, are left to
//! Android Studio.

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
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
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

    #[test]
    fn says_what_each_device_is() {
        let home = std::env::temp_dir().join(format!("cox-avd-test-{}", std::process::id()));
        write(
            &home,
            "oes-handy",
            "avd.ini.displayname=OES Handy\nhw.device.name=pixel_9\n\
             image.sysdir.1=system-images/android-35/google_apis/arm64-v8a/\n\
             hw.lcd.width=1080\nhw.lcd.height=2424\n",
        )
        .unwrap();
        write(&home, "bare", "").unwrap();
        let avds = in_folder(&home);
        std::fs::remove_dir_all(&home).ok();
        assert_eq!(avds.len(), 2);
        assert_eq!(avds[0].name, "bare");
        assert_eq!(avds[0].title, "bare");
        assert_eq!(avds[0].api, None);
        let handy = &avds[1];
        assert_eq!(handy.title, "OES Handy");
        assert_eq!(handy.model.as_deref(), Some("pixel_9"));
        assert_eq!(handy.api.as_deref(), Some("35"));
        assert_eq!(handy.display, Some((1080, 2424)));
    }

    #[test]
    fn reads_the_api_level_from_a_target_or_an_image() {
        assert_eq!(api_of("android-36").as_deref(), Some("36"));
        assert_eq!(
            api_of("system-images/android-36.1/google_apis_playstore/arm64-v8a/").as_deref(),
            Some("36.1")
        );
        assert_eq!(api_of("google_apis"), None);
    }
}
