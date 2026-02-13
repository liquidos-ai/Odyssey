//! Error types for memory operations.

/// Errors returned by memory providers and helpers.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization error.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// Regex compilation error.
    #[error("regex error: {0}")]
    Regex(String),
    /// Invalid instruction root.
    #[error("invalid instruction root: {0}")]
    InvalidRoot(String),
}
