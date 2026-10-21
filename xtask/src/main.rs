//! Development tasks for the Cox workspace, run as `cargo xtask <task>`.

use std::env;
use std::path::PathBuf;

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
    setup   Install the pinned development tools that `ci` needs
    help    Show this message
";

fn main() -> anyhow::Result<()> {
    let sh = Shell::new()?;
    match env::args().nth(1).as_deref() {
        Some("bundle") => bundle(&sh, &env::args().skip(2).collect::<Vec<_>>()),
        Some("ci") => ci(&sh),
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
    let profile = release.then_some("--release");
    // cargo's JSON output says where the executables went, and where
    // cef-dll-sys's build found the CEF distribution.
    let messages = cmd!(
        sh,
        "cargo build --bin cox --bin cox-helper {profile...} --message-format=json-render-diagnostics"
    )
    .read()?;
    let build = Build::parse(&messages)?;

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

    println!(
        "Run Cox as {}",
        app.join("Contents/MacOS").join(APP).display()
    );
    Ok(())
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
