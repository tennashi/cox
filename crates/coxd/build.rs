//! Tells coxd what its window and it say to each other, as
//! `wire::SPEAKS` reads (see `src/daemon/wire.rs`): a hash of the sources
//! that define it, so that a window of another build that says it the
//! same way can join a coxd at work.

use std::fs;
use std::path::{Path, PathBuf};

/// The sources of coxd that define what a window and coxd say: the
/// messages, how they go, and the types they carry, along with the
/// crates whose types they carry too, whole.
const SPEAKS: &[&str] = &[
    "src/daemon/wire.rs",
    "src/daemon/client.rs",
    "src/daemon/process.rs",
    "src/agent/mcp.rs",
    "src/agent/setup.rs",
    "src/chat/conversation.rs",
    "src/chat/sketch.rs",
    "src/workflow.rs",
    "../cox-core/src",
    "../cox-acp/src",
];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    speaks();
}

/// Tells coxd the FNV-1a hash of the files of [`SPEAKS`], each by its
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
