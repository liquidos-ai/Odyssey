//! Configuration models and layered config loading.
//!
//! This crate owns the Odyssey config schema, validation, and layer-merging
//! logic used by both the server and SDK.

mod error;
mod loader;
mod model;

/// Public error type returned by config loading and validation APIs.
pub use error::ConfigError;
/// Layered config types and loader options.
pub use loader::{ConfigLayer, ConfigLayerSource, LayeredConfig, LayeredConfigOptions};
/// Configuration schema models.
pub use model::*;
