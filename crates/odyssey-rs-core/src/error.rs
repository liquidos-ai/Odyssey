//! Error types for the core orchestrator crate.

use crate::types::SessionId;
use thiserror::Error;

/// Errors returned by orchestrator operations.
#[derive(Debug, Error)]
pub enum OdysseyCoreError {
    /// Session id is unknown to the orchestrator.
    #[error("unknown session: {0}")]
    UnknownSession(SessionId),
    /// Agent id is unknown to the orchestrator.
    #[error("unknown agent: {0}")]
    UnknownAgent(String),
    /// Permission enforcement failed.
    #[error("permission error: {0}")]
    Permission(String),
    /// Memory provider error.
    #[error("memory error: {0}")]
    Memory(String),
    /// State store error.
    #[error("state error: {0}")]
    State(String),
    /// Agent execution error.
    #[error("executor error: {0}")]
    Executor(String),
    /// Sandbox provider error.
    #[error("sandbox error: {0}")]
    Sandbox(String),
    /// IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Config or parsing error.
    #[error("parse error: {0}")]
    Parse(String),
}
