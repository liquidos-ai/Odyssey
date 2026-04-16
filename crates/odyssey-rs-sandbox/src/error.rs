//! Sandbox error types.

/// Errors returned by sandbox providers.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Command execution failed.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    /// Invalid sandbox configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// Access denied by sandbox policy.
    #[error("access denied: {0}")]
    AccessDenied(String),
    /// Missing dependency required by provider.
    #[error("dependency missing: {0}")]
    DependencyMissing(String),
}
