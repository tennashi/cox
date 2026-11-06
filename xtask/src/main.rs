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
            ~/Applications/Cox.app on macOS, ~/.local/bin/cox on Linux,
            with coxd started at login, by launchd or systemd
    run [--headless] [--data <dir>] [<scenario> | -- <agent command>...]
            Build Cox and run it with cox-fake-agent playing <scenario>,
            the fake agent's tour unless given, or with <agent command>.
            It keeps what lasts in <dir>, or afresh in target/debug/cox-run.
            --headless runs it on a display of its own (Xvfb, on Linux),
            with no screen; `cox --inspect` asks it how it stands
    setup   Install the pinned development tools that `ci` needs
    help    Show this message
";

fn main() -> anyhow::Result<()> {
    let sh = Shell::new()?;
    match env::args().nth(1).as_deref() {
        Some("bundle") => bundle(&sh, &env::args().skip(2).collect::<Vec<_>>()),
        Some("ci") => ci(&sh),
        Some("install") => install(&sh, &env::args().skip(2).collect::<Vec<_>>()),
        Some("run") => run(&sh, &env::args().skip(2).collect::<Vec<_>>()),
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
/// What launchd knows coxd by, as `launchctl print gui/$UID/<label>`.
const COXD_LABEL: &str = "com.github.tennashi.cox.coxd";
/// What systemd knows coxd by, as `systemctl --user status <unit>`.
const COXD_UNIT: &str = "coxd.service";
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
/// The app's icon, drawn as an SVG, which `assemble` renders into
/// `Contents/Resources/Cox.icns`.
const ICON: &str = include_str!("../../crates/cox/assets/app-icon.svg");
/// The images an `.icns` holds, each a PNG: the type macOS knows it by,
/// and its width in pixels, for 16 to 512 points at 1x and 2x.
const ICNS_IMAGES: &[(&[u8; 4], u16)] = &[
    (b"icp4", 16),
    (b"ic11", 32),
    (b"icp5", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];

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
    sh.create_dir(contents.join("Resources"))?;
    fs::write(
        contents.join("Resources").join(format!("{APP}.icns")),
        icns(ICON)?,
    )?;

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
    // The linker signs only the executables, each under a name of its own;
    // macOS lets an app post notifications only once it is signed as a
    // bundle, under the bundle's identifier. Ad hoc is enough for that.
    cmd!(sh, "codesign --force --deep --sign - {app}").run()?;
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
    start_at_login(sh, &home, &installed)?;
    println!("Run Cox as {}", installed.display());
    Ok(())
}

/// Has coxd, as the Cox at `installed` runs it, start as the human logs
/// in, so that the chats and the schedules run without a window having
/// opened first: a launch agent on macOS, which launchd loads now too,
/// starting coxd unless one runs already, and a user unit of systemd on
/// Linux, which starts at the next login. Neither starts coxd again when
/// it stops, as when a window has an outdated one stop, since the window
/// starts the new one itself.
fn start_at_login(sh: &Shell, home: &Path, installed: &Path) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        let agents = home.join("Library/LaunchAgents");
        sh.create_dir(&agents)?;
        let plist = agents.join(format!("{COXD_LABEL}.plist"));
        sh.write_file(&plist, launch_agent(installed))?;
        let uid = cmd!(sh, "id -u").read()?;
        let loaded = cmd!(sh, "launchctl print gui/{uid}/{COXD_LABEL}")
            .quiet()
            .ignore_stdout()
            .ignore_stderr()
            .run()
            .is_ok();
        if !loaded {
            cmd!(sh, "launchctl bootstrap gui/{uid} {plist}").run()?;
        }
        println!("coxd starts at login, as {}", plist.display());
    } else {
        let units = home.join(".config/systemd/user");
        sh.create_dir(&units)?;
        let unit = units.join(COXD_UNIT);
        sh.write_file(&unit, user_unit(installed))?;
        let enabled = cmd!(sh, "systemctl --user daemon-reload")
            .run()
            .and_then(|()| cmd!(sh, "systemctl --user enable {COXD_UNIT}").run());
        match enabled {
            Ok(()) => println!("coxd starts at login, as {}", unit.display()),
            Err(error) => println!(
                "coxd does not start at login, as systemd's user manager did not take {}: {error}",
                unit.display()
            ),
        }
    }
    Ok(())
}

/// The launch agent that starts coxd, of the Cox at `cox`, as the human
/// logs in.
fn launch_agent(cox: &Path) -> String {
    let cox = escape_xml(&cox.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{COXD_LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{cox}</string>
		<string>--daemon</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
"#
    )
}

/// The unit of systemd's user manager that starts coxd, of the Cox at
/// `cox`, as the human logs in.
fn user_unit(cox: &Path) -> String {
    let cox = cox
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "[Unit]\n\
         Description=coxd, which runs Cox's chats\n\
         \n\
         [Service]\n\
         ExecStart=\"{cox}\" --daemon\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// `text` as XML holds it.
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What `run` is asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Run {
    headless: bool,
    data: Option<PathBuf>,
    /// The scenario the fake agent plays, when no agent is given.
    scenario: Option<PathBuf>,
    /// The agent to run, and its arguments, in place of the fake one.
    agent: Vec<String>,
}

impl Run {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut run = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--headless" => run.headless = true,
                "--data" => {
                    run.data = Some(args.next().context("--data names a directory")?.into());
                }
                "--" => {
                    run.agent = args.by_ref().cloned().collect();
                    if run.agent.is_empty() {
                        bail!("-- is followed by the agent's command");
                    }
                }
                flag if flag.starts_with("--") => bail!("run has no {flag}"),
                scenario if run.scenario.is_none() => run.scenario = Some(scenario.into()),
                _ => bail!("run plays one scenario"),
            }
        }
        if run.scenario.is_some() && !run.agent.is_empty() {
            bail!("run plays a scenario or runs an agent, not both");
        }
        Ok(run)
    }
}

/// Builds Cox and runs it as `args` ask (see [`USAGE`]) until it quits.
fn run(sh: &Shell, args: &[String]) -> anyhow::Result<()> {
    let asked = Run::parse(args)?;
    if asked.headless && !cfg!(target_os = "linux") {
        bail!("--headless runs Cox on Xvfb, which is for Linux");
    }
    let agent = if asked.agent.is_empty() {
        let scenario = asked
            .scenario
            .unwrap_or_else(|| PathBuf::from("crates/cox-fake-agent/scenarios/tour.json"));
        let scenario = std::path::absolute(&scenario)?;
        if !scenario.is_file() {
            bail!("there is no scenario {}", scenario.display());
        }
        cmd!(sh, "cargo build --bin cox-fake-agent").run()?;
        Vec::from([String::new(), scenario.display().to_string()])
    } else {
        asked.agent
    };
    let build = build(sh, false)?;
    let debug = build
        .cox
        .parent()
        .context("the executable is in a directory")?;
    let mut agent = agent;
    if agent[0].is_empty() {
        agent[0] = debug.join("cox-fake-agent").display().to_string();
    }
    let cox = if cfg!(target_os = "macos") {
        assemble(sh, &build)?.join("Contents/MacOS").join(APP)
    } else {
        build.cox.clone()
    };
    let data = if let Some(data) = asked.data {
        std::path::absolute(data)?
    } else {
        let data = debug.join("cox-run");
        sh.remove_path(&data)?;
        data
    };
    // Chats in no project work in a folder of the run's, kept as the
    // settings screen keeps it, which the human can change from there.
    let settings = data.join("settings.json");
    if !sh.path_exists(&settings) {
        let home = data.join("home");
        sh.create_dir(&home)?;
        let home = serde_json::json!({ "home": home });
        sh.write_file(&settings, format!("{home:#}\n"))?;
    }
    let display = if asked.headless {
        Some(Display::start()?)
    } else {
        None
    };
    println!("Cox keeps what lasts in {}", data.display());
    println!("What it does is kept in {}", data.join("logs").display());
    println!(
        "Ask it how it stands, and drive it: {} --data {} --inspect [state | screenshot | key | type | click | action]",
        cox.display(),
        data.display()
    );
    if let Some(display) = &display {
        println!("It runs on DISPLAY={}", display.name);
    }
    let mut cox = cmd!(sh, "{cox} --data {data} {agent...}");
    if let Some(display) = &display {
        cox = cox.env("DISPLAY", &display.name);
    }
    cox.run()?;
    Ok(())
}

/// A display of Cox's own, which Xvfb draws in memory, until dropped.
struct Display {
    name: String,
    xvfb: std::process::Child,
}

impl Display {
    /// Starts Xvfb on the first display no other server has.
    fn start() -> anyhow::Result<Self> {
        let number = (99..200)
            .find(|number| {
                !Path::new(&format!("/tmp/.X11-unix/X{number}")).exists()
                    && !Path::new(&format!("/tmp/.X{number}-lock")).exists()
            })
            .context("every display up to :199 is taken")?;
        let name = format!(":{number}");
        let xvfb = std::process::Command::new("Xvfb")
            .args([
                name.as_str(),
                "-screen",
                "0",
                "1400x900x24",
                "-nolisten",
                "tcp",
                // Gone with its last client, Cox, however xtask ends.
                "-terminate",
            ])
            .spawn()
            .context("cannot start Xvfb; is it installed (apt-get install xvfb)?")?;
        let display = Self { name, xvfb };
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
        for _ in 0..100 {
            if socket.exists() {
                return Ok(display);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        bail!("Xvfb did not start on {}", display.name)
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        let _ = self.xvfb.kill();
        let _ = self.xvfb.wait();
    }
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

/// Renders `svg` at each size of `ICNS_IMAGES`, and answers the `.icns`
/// that holds them: `icns` and its length, then each image as its type,
/// its length and the PNG, every length counting its own header.
fn icns(svg: &str) -> anyhow::Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default())?;
    let mut images = Vec::new();
    for (kind, size) in ICNS_IMAGES {
        let mut pixmap = tiny_skia::Pixmap::new(u32::from(*size), u32::from(*size))
            .context("an icon's size is not zero")?;
        let scale = f32::from(*size) / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let png = pixmap.encode_png()?;
        images.extend_from_slice(*kind);
        images.extend_from_slice(&u32::try_from(png.len() + 8)?.to_be_bytes());
        images.extend_from_slice(&png);
    }
    let mut icns = b"icns".to_vec();
    icns.extend_from_slice(&u32::try_from(images.len() + 8)?.to_be_bytes());
    icns.extend_from_slice(&images);
    Ok(icns)
}

/// The `Info.plist` of the app, with its icon, or of a helper, which
/// stays out of the Dock instead. `MallocNanoZone` is what Chromium's own bundle sets: its
/// allocator does not get on with macOS's nano zone.
fn info_plist(name: &str, identifier: &str, helper: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let ui_element = if helper {
        "\n\t<key>LSUIElement</key>\n\t<true/>"
    } else {
        ""
    };
    let icon = if helper {
        String::new()
    } else {
        format!("\n\t<key>CFBundleIconFile</key>\n\t<string>{APP}</string>")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>{name}</string>{icon}
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
    use std::path::Path;

    use super::{Build, ICNS_IMAGES, ICON, Run, icns, launch_agent, user_unit};

    #[test]
    fn coxd_starts_at_login_as_the_installed_cox() {
        let agent = launch_agent(Path::new(
            "/Users/me/Applications/Cox.app/Contents/MacOS/Cox",
        ));
        assert!(agent.contains(
            "<array>\n\t\t<string>/Users/me/Applications/Cox.app/Contents/MacOS/Cox</string>\n\
             \t\t<string>--daemon</string>\n\t</array>"
        ));
        assert!(agent.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(!agent.contains("KeepAlive"));
        let unit = user_unit(Path::new("/home/me/.local/bin/cox"));
        assert!(unit.contains("\nExecStart=\"/home/me/.local/bin/cox\" --daemon\n"));
        assert!(unit.ends_with("[Install]\nWantedBy=default.target\n"));
    }

    #[test]
    fn the_icon_is_an_icns_of_every_size() {
        let icns = icns(ICON).unwrap();
        assert_eq!(&icns[..4], b"icns");
        let length = |at: usize| u32::from_be_bytes(icns[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(length(4), icns.len());
        let mut at = 8;
        for (kind, size) in ICNS_IMAGES {
            assert_eq!(&icns[at..at + 4], *kind);
            let png = &icns[at + 8..at + length(at + 4)];
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
            // The width and the height, in the PNG's IHDR chunk.
            assert_eq!(
                png[16..24],
                [[0, 0], size.to_be_bytes(), [0, 0], size.to_be_bytes()].concat()
            );
            at += length(at + 4);
        }
        assert_eq!(at, icns.len());
    }

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

    fn parse(args: &[&str]) -> anyhow::Result<Run> {
        Run::parse(&args.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn run_plays_a_scenario_or_runs_an_agent() {
        assert_eq!(parse(&[]).unwrap(), Run::default());
        assert_eq!(
            parse(&["--headless", "--data", "d", "s.json"]).unwrap(),
            Run {
                headless: true,
                data: Some("d".into()),
                scenario: Some("s.json".into()),
                agent: Vec::new(),
            }
        );
        assert_eq!(
            parse(&["--", "npx", "--yes", "agent"]).unwrap().agent,
            ["npx", "--yes", "agent"]
        );
        assert!(parse(&["s.json", "--", "agent"]).is_err());
        assert!(parse(&["a.json", "b.json"]).is_err());
        assert!(parse(&["--data"]).is_err());
        assert!(parse(&["--fast"]).is_err());
        assert!(parse(&["--"]).is_err());
    }
}
