//! On Linux, `cox` links `libcef.so`, and CEF looks for its resources next
//! to that library, so the executables need to find it at run time. The
//! `cef` crate's build copies the whole distribution next to the
//! executables (`target/<profile>/`), but pointing the dynamic loader there
//! would also hand gpui CEF's own `libvulkan.so.1`, which cannot draw to a
//! window. So the loader is pointed at a mirror of the distribution, made
//! of links, that leaves that one file out.
//!
//! `cargo xtask install` puts `cox` in `~/.local/bin` and the same files
//! in `~/.local/lib/cox`, so the loader looks there first, beside the
//! executable. Next to `target/<profile>/cox` there is no such directory.
//!
//! macOS needs none of this: there, nothing links CEF at build time, and
//! `cargo xtask bundle` puts the framework where the app loads it from.
//!
//! The browser is `cox-browser`'s, but the loader's path is the
//! executables' to set: what a build script says of linking goes to its
//! own package's executables only, so the script is here, and `cox`
//! depends on `cef-dll-sys` as well, which tells the build scripts of its
//! direct dependents where CEF is.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    // `cef-dll-sys` tells its dependents where the CEF distribution is.
    let cef_dir =
        PathBuf::from(env::var("DEP_CEF_DLL_WRAPPER_CEF_DIR").expect("cef-dll-sys built"));
    let lib_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("cef");
    if lib_dir.exists() {
        fs::remove_dir_all(&lib_dir).expect("the old mirror is removed");
    }
    fs::create_dir_all(&lib_dir).expect("the mirror directory is created");
    for entry in fs::read_dir(&cef_dir).expect("the CEF distribution is readable") {
        let entry = entry.expect("the CEF distribution is readable");
        if entry.file_name() == "libvulkan.so.1" {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), lib_dir.join(entry.file_name()))
            .expect("the CEF file is linked");
    }
    println!("cargo::rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/cox");
    println!("cargo::rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
}
