//! Public SDK surface for Odyssey.
//!
//! This crate re-exports the core building blocks and provides a small
//! initialization helper to keep consumer setup consistent.

/// Re-export for convenience.
pub use odyssey_rs_config as config;
pub use odyssey_rs_core as core;
/// Re-export for convenience.
pub use odyssey_rs_memory as memory;
/// Re-export for convenience.
pub use odyssey_rs_protocol as protocol;

#[inline]
/// Initialize logging using env_logger if the "logging" feature is enabled.
///
/// This is a no-op if the feature is not enabled. Binaries are still expected
/// to call this early in startup to ensure log output is wired up.
pub fn init_logging() {
    #[cfg(feature = "logging")]
    {
        let _ = env_logger::try_init();
    }
}
