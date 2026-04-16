//! Wire protocol types for Odyssey events, Requests, and common types.

mod skill;
mod tool;

pub use skill::{SkillProvider, SkillSummary};
pub use tool::ToolError;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

/// Unique identifier for a session.
pub type SessionId = Uuid;
/// Unique identifier for a turn.
pub type TurnId = Uuid;
/// Unique identifier for a tool call.
pub type ToolCallId = Uuid;
/// Unique identifier for an exec stream.
pub type ExecId = Uuid;

/// Wrapper for client submissions into the submission queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionEnvelope {
    /// Unique id for the submission.
    pub id: Uuid,
    /// Session id for the submission.
    pub session_id: SessionId,
    /// Timestamp when the submission was created.
    pub created_at: DateTime<Utc>,
    /// Submission payload content.
    pub payload: SubmissionPayload,
}

/// All submission operations that a client can enqueue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum SubmissionPayload {
    /// Submit a user message to start a turn.
    UserMessage { content: String },
    /// Override turn context defaults without user input.
    OverrideTurnContext { context: TurnContextOverride },
    /// Cancel an in-flight turn.
    CancelTurn { turn_id: TurnId },
}

/// Wrapper for events emitted by the event queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMsg {
    /// Unique id for the event.
    pub id: Uuid,
    /// Session id associated with the event.
    pub session_id: SessionId,
    /// Timestamp when the event was created.
    pub created_at: DateTime<Utc>,
    /// Event payload content.
    pub payload: EventPayload,
}

/// All events emitted during orchestration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum EventPayload {
    /// Turn lifecycle started.
    TurnStarted {
        turn_id: TurnId,
        context: TurnContext,
    },
    /// Turn lifecycle completed.
    TurnCompleted { turn_id: TurnId, message: String },
    /// Streaming response delta from the agent.
    AgentMessageDelta { turn_id: TurnId, delta: String },
    /// Streaming reasoning delta from the agent.
    ReasoningDelta { turn_id: TurnId, delta: String },
    /// Separator between reasoning sections.
    ReasoningSectionBreak { turn_id: TurnId },
    /// Tool call execution started.
    ToolCallStarted {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        tool_name: String,
        arguments: Value,
    },
    /// Tool call output delta.
    ToolCallDelta {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        delta: Value,
    },
    /// Tool call execution completed.
    ToolCallFinished {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        result: Value,
        success: bool,
    },
    /// Execution command started.
    ExecCommandBegin {
        turn_id: TurnId,
        exec_id: ExecId,
        command: Vec<String>,
        cwd: Option<String>,
    },
    /// Execution output delta.
    ExecCommandOutputDelta {
        turn_id: TurnId,
        exec_id: ExecId,
        stream: ExecStream,
        delta: String,
    },
    /// Execution command finished.
    ExecCommandEnd {
        turn_id: TurnId,
        exec_id: ExecId,
        exit_code: i32,
    },
    /// Permission request emitted for approval.
    PermissionRequested {
        turn_id: TurnId,
        request_id: Uuid,
        action: PermissionAction,
        request: PermissionRequest,
    },
    /// Permission decision resolved.
    ApprovalResolved {
        turn_id: TurnId,
        request_id: Uuid,
        decision: ApprovalDecision,
    },
    /// Plan update broadcast.
    PlanUpdate { turn_id: TurnId, plan: Value },
    /// Error event for the session or turn.
    Error {
        turn_id: Option<TurnId>,
        message: String,
    },
}

/// Execution output stream selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStream {
    /// Standard output stream.
    Stdout,
    /// Standard error stream.
    Stderr,
}

/// Turn-scoped execution context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnContext {
    /// Working directory for tool execution.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Model spec used for the turn.
    #[serde(default)]
    pub model: Option<ModelSpec>,
    /// Sandbox mode for tool execution.
    #[serde(default)]
    pub sandbox_mode: Option<SandboxMode>,
    /// Approval policy override for tools.
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    /// Additional metadata for the turn.
    #[serde(default = "empty_json_object")]
    pub metadata: Value,
}

impl TurnContext {
    /// Apply a partial override onto this context.
    pub fn apply_override(&mut self, override_ctx: &TurnContextOverride) {
        if override_ctx.cwd.is_some() {
            self.cwd = override_ctx.cwd.clone();
        }
        if override_ctx.model.is_some() {
            self.model = override_ctx.model.clone();
        }
        if override_ctx.sandbox_mode.is_some() {
            self.sandbox_mode = override_ctx.sandbox_mode;
        }
        if override_ctx.approval_policy.is_some() {
            self.approval_policy = override_ctx.approval_policy;
        }
        let Some(override_map) = override_ctx.metadata.as_object() else {
            return;
        };
        if override_map.is_empty() {
            return;
        }
        match self.metadata.as_object_mut() {
            Some(target) => {
                for (key, value) in override_map {
                    target.insert(key.clone(), value.clone());
                }
            }
            None => {
                self.metadata = override_ctx.metadata.clone();
            }
        }
    }
}

/// Partial override of turn context fields.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnContextOverride {
    /// Override working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Override model spec.
    #[serde(default)]
    pub model: Option<ModelSpec>,
    /// Override sandbox mode.
    #[serde(default)]
    pub sandbox_mode: Option<SandboxMode>,
    /// Override approval policy.
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    /// Override metadata fields.
    #[serde(default = "empty_json_object")]
    pub metadata: Value,
}

/// Model specification used for a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Provider identifier (e.g., openai).
    pub provider: String,
    /// Model name under the provider.
    pub name: String,
}

/// Approval policy for tool execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// Treat all tools as untrusted and require approval.
    Untrusted,
    /// Require approval only on failure conditions.
    OnFailure,
    /// Require approval on explicit request.
    OnRequest,
    /// Never require approval.
    Never,
}

/// Sandbox policy presets.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Read-only access to the workspace.
    ReadOnly,
    /// Allow writes within the workspace root.
    WorkspaceWrite,
    /// Full access without sandboxing guarantees.
    DangerFullAccess,
}

/// Request for a permission decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum PermissionRequest {
    /// Tool invocation permission.
    Tool { name: String },
    /// Workspace path access request.
    Path { path: String, mode: PathAccess },
    /// External path access request.
    ExternalPath { path: String, mode: PathAccess },
    /// Command execution request.
    Command { argv: Vec<String> },
}

/// Path access mode used in permission checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathAccess {
    /// Read access.
    Read,
    /// Write access.
    Write,
    /// Execute access.
    Execute,
}

/// Policy action resolved for a permission request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    /// Allow the action.
    Allow,
    /// Deny the action.
    Deny,
    /// Ask for explicit approval.
    #[default]
    Ask,
}

/// Decision returned by a user or policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Allow the action once.
    AllowOnce,
    /// Always allow the action for this session.
    AllowAlways,
    /// Deny the action.
    Deny,
}

/// Sink interface for orchestrator and tool events.
pub trait EventSink: Send + Sync {
    /// Emit an event to downstream listeners.
    fn emit(&self, event: EventMsg);
}

/// Default metadata value for empty JSON objects.
fn empty_json_object() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn turn_context_override_merges_metadata() {
        let mut ctx = TurnContext {
            cwd: Some("/workspace".to_string()),
            model: Some(ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
            }),
            sandbox_mode: Some(SandboxMode::ReadOnly),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            metadata: json!({ "existing": 1 }),
        };
        let override_ctx = TurnContextOverride {
            cwd: Some("/override".to_string()),
            approval_policy: Some(ApprovalPolicy::Never),
            metadata: json!({ "extra": true }),
            ..TurnContextOverride::default()
        };
        ctx.apply_override(&override_ctx);

        assert_eq!(ctx.cwd, Some("/override".to_string()));
        assert_eq!(ctx.approval_policy, Some(ApprovalPolicy::Never));
        assert_eq!(ctx.metadata, json!({ "existing": 1, "extra": true }));
    }

    #[test]
    fn event_payload_round_trips_through_json() {
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            created_at: Utc::now(),
            payload: EventPayload::ToolCallFinished {
                turn_id: Uuid::new_v4(),
                tool_call_id: Uuid::new_v4(),
                result: json!({ "ok": true }),
                success: true,
            },
        };
        let encoded = serde_json::to_value(&event).expect("serialize");
        let decoded: EventMsg = serde_json::from_value(encoded.clone()).expect("deserialize");
        let decoded_value = serde_json::to_value(decoded).expect("serialize decoded");
        assert_eq!(decoded_value, encoded);
    }
}
