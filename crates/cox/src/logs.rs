//! What Cox says of itself as it runs: what went wrong, and, for whoever
//! debugs it, what it did. Cox says it through `tracing`; gpui and the
//! other crates that say it through `log` reach the same place.
//!
//! It goes to standard error, as `RUST_LOG` asks, and only the errors
//! unless it asks for more.

use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// Starts taking what Cox says, for the rest of the process.
///
/// # Panics
///
/// When something else took it already.
pub fn init() {
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::ERROR.into())
                .from_env_lossy(),
        );
    tracing_subscriber::registry().with(stderr).init();
}
