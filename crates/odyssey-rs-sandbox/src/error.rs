use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("unsupported sandbox: {0}")]
    Unsupported(String),
    #[error("access denied: {0}")]
    AccessDenied(String),
    #[error("dependency missing: {0}")]
    DependencyMissing(String),
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
}
