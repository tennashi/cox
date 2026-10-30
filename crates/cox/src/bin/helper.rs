//! The executable behind Cox's helper apps on macOS.
//!
//! Chromium runs its renderer, GPU and other work in processes of their
//! own. On Linux those run `cox` itself again, but on macOS each one is a
//! separate app bundle under `Cox.app/Contents/Frameworks`, all with this
//! executable inside; `cargo xtask bundle` puts them there. CEF tells the
//! process what to be through the command line, so all this does is load
//! CEF and hand over.

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(cox_browser::run_helper());
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("cox-helper only has work to do on macOS; see src/bin/helper.rs");
    std::process::exit(2);
}
