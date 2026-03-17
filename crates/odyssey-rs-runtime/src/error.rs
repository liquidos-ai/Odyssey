use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("bundle error: {0}")]
    Bundle(#[from] odyssey_rs_bundle::BundleError),
    #[error("manifest error: {0}")]
    Manifest(#[from] odyssey_rs_manifest::ManifestError),
    #[error("tool error: {0}")]
    Tool(#[from] odyssey_rs_tools::ToolError),
    #[error("sandbox error: {0}")]
    Sandbox(#[from] odyssey_rs_sandbox::SandboxError),
    #[error("io error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("session not found: {0}")]
    UnknownSession(String),
    #[error("approval request not found: {0}")]
    UnknownApproval(String),
    #[error("executor error: {0}")]
    Executor(String),
    #[error("template error: {0}")]
    Template(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}
