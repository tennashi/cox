//! An Android emulator on the stage: Cox starts one, or finds one already
//! running, and talks to it over its gRPC service, as Android Studio's
//! "Running Devices" does, to show its display and send it the human's
//! touches and keys, with no window of the emulator's own.
//!
//! - [`sdk`] finds the emulator's program, and [`avd`] the virtual
//!   devices it runs, and what each is.
//! - [`running`] finds the emulators running, by the files each writes
//!   of itself as it starts.
//! - [`device`] starts or reaches one emulator, streams its display and
//!   sends it input.
//! - [`proto`] is the part of the emulator's gRPC service Cox uses, which
//!   `cox-fake-emulator` serves too.

pub mod avd;
pub mod device;
pub mod running;
pub mod sdk;

pub mod proto {
    #![expect(
        clippy::pedantic,
        clippy::allow_attributes,
        clippy::allow_attributes_without_reason,
        non_camel_case_types,
        reason = "the code tonic generates from the emulator's .proto"
    )]
    tonic::include_proto!("android.emulation.control");
}
