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
//!
//! It also tells `cox` what its window and coxd say to each other, as
//! `wire::SPEAKS` reads (see `src/daemon/wire.rs`): a hash of the sources
//! that define it, so that a window of another build that says it the
//! same way can join a coxd at work.

use std::path::{Path, PathBuf};
use std::{env, fs};

/// The sources of `cox` that define what a window and coxd say: the
/// messages, how they go, and the types they carry, along with the
/// crates whose types they carry too, whole.
const SPEAKS: &[&str] = &[
    "src/daemon/wire.rs",
    "src/daemon/client.rs",
    "src/daemon/process.rs",
    "../coxd/src/agent/mcp.rs",
    "../coxd/src/agent/setup.rs",
    "../coxd/src/chat/conversation.rs",
    "../coxd/src/chat/sketch.rs",
    "../coxd/src/workflow.rs",
    "../cox-core/src",
    "../cox-acp/src",
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    speaks();
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

/// Tells `cox` the FNV-1a hash of the files of [`SPEAKS`], each by its
/// path and its bytes, in the order of their paths, as `COX_SPEAKS`.
fn speaks() {
    let mut files = Vec::new();
    for source in SPEAKS {
        println!("cargo::rerun-if-changed={source}");
        files_of(Path::new(source), &mut files);
    }
    files.sort();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for file in files {
        let bytes = fs::read(&file).expect("a source of the wire is readable");
        let path = file.to_string_lossy();
        for &byte in path.as_bytes().iter().chain(&[0]).chain(&bytes).chain(&[0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    println!("cargo::rustc-env=COX_SPEAKS={hash:016x}");
}

/// Adds `path` to `files`, or the files under it, if it is a folder.
fn files_of(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).expect("a source folder of the wire is readable") {
            files_of(
                &entry
                    .expect("a source folder of the wire is readable")
                    .path(),
                files,
            );
        }
    } else {
        files.push(path.to_owned());
    }
}
