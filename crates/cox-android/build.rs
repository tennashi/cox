//! Compiles the part of the Android emulator's gRPC service that Cox uses
//! (`proto/emulator_controller.proto`) into its Rust: the messages, the
//! client Cox talks to an emulator with, and the server
//! `cox-fake-emulator` answers as one.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/emulator_controller.proto";
    println!("cargo:rerun-if-changed={proto}");
    let files = protox::compile([proto], ["proto"])?;
    tonic_prost_build::configure().compile_fds(files)?;
    Ok(())
}
