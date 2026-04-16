//! Error types for config loading and validation.

use thiserror::Error;

/// Errors returned while loading or validating config.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Reading a config file failed.
    #[error("failed to read config: {0}")]
    ReadFailed(#[from] std::io::Error),
    /// Parsing a config file failed.
    #[error("failed to parse config: {0}")]
    ParseFailed(#[from] json5::Error),
    /// Converting JSON values failed.
    #[error("failed to decode config: {0}")]
    DecodeFailed(#[from] serde_json::Error),
    /// A specific field failed validation.
    #[error("invalid config at {path}: {message}")]
    InvalidField { path: String, message: String },
    /// Generic validation failure.
    #[error("invalid config: {0}")]
    Invalid(String),
}
