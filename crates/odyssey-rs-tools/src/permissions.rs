//! Permission checking interfaces for tools.

use async_trait::async_trait;
use odyssey_rs_protocol::PermissionRequest;
use odyssey_rs_protocol::ToolError;
use uuid::Uuid;

/// Context for a permission request.
#[derive(Debug, Clone)]
pub struct PermissionContext {
    /// Session id for the request.
    pub session_id: Uuid,
    /// Agent id for the request.
    pub agent_id: String,
    /// Tool name that originated the request, if any.
    pub tool_name: Option<String>,
    /// Optional turn id for the request.
    pub turn_id: Option<Uuid>,
}

/// Outcome of a permission check.
#[derive(Debug, Clone)]
pub struct PermissionOutcome {
    /// Whether the request is allowed.
    pub allowed: bool,
    /// Optional denial reason.
    pub reason: Option<String>,
}

/// Permission checker interface used by tool contexts.
#[async_trait]
pub trait PermissionChecker: Send + Sync {
    /// Authorize a request and return an outcome.
    async fn authorize(
        &self,
        ctx: &PermissionContext,
        request: PermissionRequest,
    ) -> Result<PermissionOutcome, ToolError>;
}
