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
    use cef::args::Args;

    cox::browser::load_framework(cox::browser::Process::Helper);
    cef::api_hash(cef::sys::CEF_API_VERSION, 0);
    let args = Args::new();
    let code = cef::execute_process(
        Some(args.as_main_args()),
        None::<&mut cef::App>,
        std::ptr::null_mut(),
    );
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("cox-helper only has work to do on macOS; see src/bin/helper.rs");
    std::process::exit(2);
}
