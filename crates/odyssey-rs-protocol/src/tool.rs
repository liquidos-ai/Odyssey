/// Errors returned by tools and tool adapters.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// Tool name was not found in registry.
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    /// Tool received invalid arguments.
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    /// Tool execution failed.
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    /// Tool execution was denied by permissions.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
}
