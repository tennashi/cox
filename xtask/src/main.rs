//! Development tasks for the Cox workspace, run as `cargo xtask <task>`.

use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, bail};
use xshell::{Shell, cmd};

/// Development tools that `setup` installs, pinned to exact versions so that
/// every machine and CI run the same ones.
const TOOLS: &[(&str, &str)] = &[("cargo-deny", "0.20.2")];

const USAGE: &str = "\
Usage: cargo xtask <task>

Tasks:
    bundle [--release]
            Build Cox and assemble Cox.app, the macOS app bundle, next to
            the executables under target/
    ci      Run every check that CI runs: format, lint, test and dependencies
    install
            Build Cox for release and install it apart from target/:
            ~/Applications/Cox.app on macOS, ~/.local/bin/cox on Linux
    setup   Install the pinned development tools that `ci` needs
    help    Show this message
";

fn main() -> anyhow::Result<()> {
    let sh = Shell::new()?;
    match env::args().nth(1).as_deref() {
        Some("bundle") => bundle(&sh, &env::args().skip(2).collect::<Vec<_>>()),
        Some("ci") => ci(&sh),
        Some("install") => install(&sh, &env::args().skip(2).collect::<Vec<_>>()),
        Some("setup") => setup(&sh),
        None | Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(task) => {
            eprint!("{USAGE}");
            bail!("unknown task: {task}")
        }
    }
}

/// Runs the checks in order and stops at the first one that fails.
fn ci(sh: &Shell) -> anyhow::Result<()> {
    cmd!(sh, "cargo fmt --all --check").run()?;
    cmd!(
        sh,
        "cargo clippy --workspace --all-targets -- --deny warnings"
    )
    .run()?;
    cmd!(sh, "cargo test --workspace").run()?;
    cmd!(sh, "cargo deny check")
        .run()
        .context("cargo-deny failed; if it is not installed, run `cargo xtask setup`")?;
    Ok(())
}

/// Installs each tool in `TOOLS`. Cargo skips a tool whose pinned version is
/// already installed.
fn setup(sh: &Shell) -> anyhow::Result<()> {
    for (name, version) in TOOLS {
        let spec = format!("{name}@{version}");
        cmd!(sh, "cargo install --locked {spec}").run()?;
    }
    Ok(())
}

/// The name the app, its executable and its helpers go by. CEF finds the
/// helpers from the executable's name, so the three agree.
const APP: &str = "Cox";
const BUNDLE_ID: &str = "com.github.tennashi.cox";
/// What CEF appends to the app's name for each kind of helper process,
/// paired with what the bundle identifier gets.
const HELPERS: &[(&str, &str)] = &[
    ("", "helper"),
    (" (GPU)", "helper.gpu"),
    (" (Renderer)", "helper.renderer"),
    (" (Plugin)", "helper.plugin"),
    (" (Alerts)", "helper.alerts"),
];
const FRAMEWORK: &str = "Chromium Embedded Framework.framework";

/// Builds `cox` and `cox-helper`, then lays them out with CEF the way
/// macOS and CEF expect: the framework under `Contents/Frameworks`, and
/// one helper app beside it for each kind of process CEF starts.
fn bundle(sh: &Shell, args: &[String]) -> anyhow::Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("bundle assembles the macOS app; run it on a Mac");
    }
    let release = match args {
        [] => false,
        [flag] if flag == "--release" => true,
        _ => bail!("usage: cargo xtask bundle [--release]"),
    };
    let app = assemble(sh, &build(sh, release)?)?;
    println!(
        "Run Cox as {}",
        app.join("Contents/MacOS").join(APP).display()
    );
    Ok(())
}

/// Lays out `Cox.app` next to the executables of `build`, and answers
/// where it is.
fn assemble(sh: &Shell, build: &Build) -> anyhow::Result<PathBuf> {
    let app = build
        .cox
        .parent()
        .context("the executable is in a directory")?
        .join(format!("{APP}.app"));
    sh.remove_path(&app)?;
    let contents = app.join("Contents");
    let frameworks = contents.join("Frameworks");
    sh.create_dir(&frameworks)?;
    sh.write_file(
        contents.join("Info.plist"),
        info_plist(APP, BUNDLE_ID, false),
    )?;
    sh.create_dir(contents.join("MacOS"))?;
    sh.copy_file(&build.cox, contents.join("MacOS").join(APP))?;

    let framework = build.cef_dir.join(FRAMEWORK);
    // cp keeps what the framework is made of (links, modes) as it is.
    cmd!(sh, "cp -R {framework} {frameworks}/").run()?;

    for (suffix, id) in HELPERS {
        let name = format!("{APP} Helper{suffix}");
        let contents = frameworks.join(format!("{name}.app")).join("Contents");
        sh.create_dir(&contents)?;
        sh.write_file(
            contents.join("Info.plist"),
            info_plist(&name, &format!("{BUNDLE_ID}.{id}"), true),
        )?;
        sh.create_dir(contents.join("MacOS"))?;
        sh.copy_file(&build.helper, contents.join("MacOS").join(&name))?;
    }
    Ok(app)
}

/// What Cox needs of the CEF distribution on Linux when it runs: CEF and
/// the libraries it loads, and its resources. Not CEF's `libvulkan.so.1`,
/// which `cox` must not find (see `crates/cox/build.rs`), nor its sandbox,
/// which Cox runs without.
const CEF_ON_LINUX: &[&str] = &[
    "libcef.so",
    "libEGL.so",
    "libGLESv2.so",
    "libvk_swiftshader.so",
    "vk_swiftshader_icd.json",
    "icudtl.dat",
    "v8_context_snapshot.bin",
    "chrome_100_percent.pak",
    "chrome_200_percent.pak",
    "resources.pak",
    "locales",
];

/// Builds Cox for release and installs it where the Cox in use runs from,
/// apart from `target/`, so that building and trying Cox leaves it be:
/// `~/Applications/Cox.app` on macOS, and on Linux `~/.local/bin/cox`
/// with CEF in `~/.local/lib/cox`, where `cox` looks for it first.
///
/// A Cox already running there keeps running; it runs the new version
/// once it is started again.
fn install(sh: &Shell, args: &[String]) -> anyhow::Result<()> {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        bail!("install knows where Cox goes on macOS and Linux only");
    }
    if !args.is_empty() {
        bail!("usage: cargo xtask install");
    }
    let home = PathBuf::from(env::var_os("HOME").context("HOME is set")?);
    let build = build(sh, true)?;
    let installed = if cfg!(target_os = "macos") {
        let app = assemble(sh, &build)?;
        let applications = home.join("Applications");
        sh.create_dir(&applications)?;
        let installed = applications.join(format!("{APP}.app"));
        // cp keeps what the framework is made of, as `assemble` does.
        replace(sh, &installed, |staged| {
            Ok(cmd!(sh, "cp -R {app} {staged}").run()?)
        })?;
        installed.join("Contents/MacOS").join(APP)
    } else {
        replace(sh, &home.join(".local/lib/cox"), |staged| {
            sh.create_dir(staged)?;
            for name in CEF_ON_LINUX {
                let file = build.cef_dir.join(name);
                cmd!(sh, "cp -R {file} {staged}/").run()?;
            }
            Ok(())
        })?;
        let bin = home.join(".local/bin");
        sh.create_dir(&bin)?;
        let installed = bin.join("cox");
        replace(sh, &installed, |staged| {
            Ok(sh.copy_file(&build.cox, staged)?)
        })?;
        installed
    };
    println!("Run Cox as {}", installed.display());
    Ok(())
}

/// Puts what `make` makes at `path`, in place of what is there. `make`
/// makes it beside `path`, so `path` holds either the old one or the new
/// one, whole, all but for a moment.
fn replace(
    sh: &Shell,
    path: &Path,
    make: impl FnOnce(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let name = path
        .file_name()
        .context("the path ends in a name")?
        .to_string_lossy();
    let staged = path.with_file_name(format!(".{name}.new"));
    let old = path.with_file_name(format!(".{name}.old"));
    sh.remove_path(&staged)?;
    sh.remove_path(&old)?;
    make(&staged)?;
    if sh.path_exists(path) {
        fs::rename(path, &old)?;
    }
    fs::rename(&staged, path)?;
    sh.remove_path(&old)?;
    Ok(())
}

/// Builds `cox` and `cox-helper`, for release or not.
fn build(sh: &Shell, release: bool) -> anyhow::Result<Build> {
    let profile = release.then_some("--release");
    // cargo's JSON output says where the executables went, and where
    // cef-dll-sys's build found the CEF distribution.
    let messages = cmd!(
        sh,
        "cargo build --bin cox --bin cox-helper {profile...} --message-format=json-render-diagnostics"
    )
    .read()?;
    Build::parse(&messages)
}

/// What `cargo build --message-format=json` said that `bundle` needs.
struct Build {
    cox: PathBuf,
    helper: PathBuf,
    cef_dir: PathBuf,
}

impl Build {
    fn parse(messages: &str) -> anyhow::Result<Self> {
        let mut cox = None;
        let mut helper = None;
        let mut cef_dir = None;
        for line in messages.lines() {
            let message: serde_json::Value = serde_json::from_str(line)?;
            match message["reason"].as_str() {
                Some("compiler-artifact") => {
                    let Some(executable) = message["executable"].as_str() else {
                        continue;
                    };
                    match message["target"]["name"].as_str() {
                        Some("cox") => cox = Some(PathBuf::from(executable)),
                        Some("cox-helper") => helper = Some(PathBuf::from(executable)),
                        _ => {}
                    }
                }
                Some("build-script-executed")
                    if message["package_id"]
                        .as_str()
                        .is_some_and(|id| id.contains("cef-dll-sys")) =>
                {
                    // The build script's `cargo::metadata=CEF_DIR=...` is for
                    // other build scripts and not in the message, but it also
                    // makes the distribution a library search path.
                    let paths = message["linked_paths"]
                        .as_array()
                        .context("linked_paths is a list")?;
                    cef_dir = paths
                        .iter()
                        .filter_map(|path| path.as_str()?.strip_prefix("native="))
                        .map(PathBuf::from)
                        .find(|path| {
                            path.file_name()
                                .and_then(|name| name.to_str())
                                .is_some_and(|name| name.starts_with("cef_"))
                        });
                }
                _ => {}
            }
        }
        Ok(Self {
            cox: cox.context("cargo built cox")?,
            helper: helper.context("cargo built cox-helper")?,
            cef_dir: cef_dir.context("cef-dll-sys's build said where CEF is")?,
        })
    }
}

/// The `Info.plist` of the app, or of a helper, which also stays out of
/// the Dock. `MallocNanoZone` is what Chromium's own bundle sets: its
/// allocator does not get on with macOS's nano zone.
fn info_plist(name: &str, identifier: &str, helper: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let ui_element = if helper {
        "\n\t<key>LSUIElement</key>\n\t<true/>"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>{name}</string>
	<key>CFBundleIdentifier</key>
	<string>{identifier}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>{name}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSEnvironment</key>
	<dict>
		<key>MallocNanoZone</key>
		<string>0</string>
	</dict>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>{ui_element}
</dict>
</plist>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::Build;

    #[test]
    fn the_build_is_read_from_cargo_messages() {
        let messages = concat!(
            r#"{"reason":"compiler-artifact","target":{"name":"cox"},"executable":null}"#,
            "\n",
            r#"{"reason":"build-script-executed","package_id":"registry+https://github.com/rust-lang/crates.io-index#cef-dll-sys@152.3.0+152.0.6","linked_paths":["native=/wrapper/build/libcef_dll_wrapper","native=/cef/cef_macos_aarch64"],"env":[]}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"name":"cox-helper"},"executable":"/target/debug/cox-helper"}"#,
            "\n",
            r#"{"reason":"compiler-artifact","target":{"name":"cox"},"executable":"/target/debug/cox"}"#,
            "\n",
            r#"{"reason":"build-finished","success":true}"#,
        );
        let build = Build::parse(messages).unwrap();
        assert_eq!(build.cox.to_str(), Some("/target/debug/cox"));
        assert_eq!(build.helper.to_str(), Some("/target/debug/cox-helper"));
        assert_eq!(build.cef_dir.to_str(), Some("/cef/cef_macos_aarch64"));
    }

    #[test]
    fn a_build_without_cef_is_refused() {
        let messages = r#"{"reason":"compiler-artifact","target":{"name":"cox"},"executable":"/target/debug/cox"}"#;
        assert!(Build::parse(messages).is_err());
    }
}
