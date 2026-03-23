//! Shared protocol types for Odyssey runtime surfaces.

mod skill;
mod tool;

pub use skill::{SkillProvider, SkillSummary};
pub use tool::ToolError;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

pub type SessionId = Uuid;
pub type TurnId = Uuid;
pub type ToolCallId = Uuid;
pub type ExecId = Uuid;

pub use autoagents_protocol::Task;
pub use autoagents_protocol::{Event as AutoAgentsEvent, StreamChunk as AutoAgentsStreamChunk};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct BundleRef {
    pub reference: String,
}

impl BundleRef {
    pub fn new(reference: impl Into<String>) -> Self {
        Self {
            reference: reference.into(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.reference
    }
}

impl From<String> for BundleRef {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for BundleRef {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for BundleRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.reference)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub agent_id: String,
    pub message_count: usize,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub agent_id: String,
    pub bundle_ref: String,
    pub model_id: String,
    pub created_at: DateTime<Utc>,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    pub bundle_ref: BundleRef,
    #[serde(default)]
    pub model: Option<ModelSpec>,
    #[serde(default = "empty_json_object")]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionFilter {
    #[serde(default)]
    pub bundle_ref: Option<BundleRef>,
}

impl From<&str> for SessionSpec {
    fn from(value: &str) -> Self {
        Self {
            bundle_ref: BundleRef::from(value),
            model: None,
            metadata: empty_json_object(),
        }
    }
}

impl From<String> for SessionSpec {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub request_id: Uuid,
    pub session_id: SessionId,
    pub input: Task,
    #[serde(default)]
    pub turn_context: Option<TurnContextOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionHandle {
    pub session_id: SessionId,
    pub turn_id: TurnId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmissionEnvelope {
    pub id: Uuid,
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
    pub payload: SubmissionPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum SubmissionPayload {
    UserMessage { content: String },
    OverrideTurnContext { context: TurnContextOverride },
    CancelTurn { turn_id: TurnId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMsg {
    pub id: Uuid,
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum EventPayload {
    TurnStarted {
        turn_id: TurnId,
        context: TurnContext,
    },
    TurnCompleted {
        turn_id: TurnId,
        message: String,
    },
    AgentMessageDelta {
        turn_id: TurnId,
        delta: String,
    },
    ReasoningDelta {
        turn_id: TurnId,
        delta: String,
    },
    ReasoningSectionBreak {
        turn_id: TurnId,
    },
    ToolCallStarted {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        tool_name: String,
        arguments: Value,
    },
    ToolCallDelta {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        delta: Value,
    },
    ToolCallFinished {
        turn_id: TurnId,
        tool_call_id: ToolCallId,
        result: Value,
        success: bool,
    },
    ExecCommandBegin {
        turn_id: TurnId,
        exec_id: ExecId,
        command: Vec<String>,
        cwd: Option<String>,
    },
    ExecCommandOutputDelta {
        turn_id: TurnId,
        exec_id: ExecId,
        stream: ExecStream,
        delta: String,
    },
    ExecCommandEnd {
        turn_id: TurnId,
        exec_id: ExecId,
        exit_code: i32,
    },
    PermissionRequested {
        turn_id: TurnId,
        request_id: Uuid,
        action: PermissionAction,
        request: PermissionRequest,
    },
    ApprovalResolved {
        turn_id: TurnId,
        request_id: Uuid,
        decision: ApprovalDecision,
    },
    PlanUpdate {
        turn_id: TurnId,
        plan: Value,
    },
    Error {
        turn_id: Option<TurnId>,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnContext {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<ModelSpec>,
    #[serde(default)]
    pub sandbox_mode: Option<SandboxMode>,
    #[serde(default)]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(default = "empty_json_object")]
    pub metadata: Value,
}

impl TurnContext {
    pub fn apply_override(&mut self, override_ctx: &TurnContextOverride) {
        if override_ctx.cwd.is_some() {
            self.cwd = override_ctx.cwd.clone();
        }
        if override_ctx.model.is_some() {
            self.model = override_ctx.model.clone();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurnContextOverride {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<ModelSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub provider: String,
    pub name: String,
    //Provider Config
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Untrusted,
    OnFailure,
    OnRequest,
    Never,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "payload")]
pub enum PermissionRequest {
    Tool { name: String },
    Path { path: String, mode: PathAccess },
    ExternalPath { path: String, mode: PathAccess },
    Command { argv: Vec<String> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathAccess {
    Read,
    Write,
    Execute,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Allow,
    Deny,
    #[default]
    Ask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: EventMsg);
}

fn empty_json_object() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn turn_context_override() {
        let mut ctx = TurnContext {
            cwd: Some("/workspace".to_string()),
            model: Some(ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
                config: None,
            }),
            sandbox_mode: Some(SandboxMode::ReadOnly),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            metadata: json!({ "existing": 1 }),
        };
        let override_ctx = TurnContextOverride {
            cwd: Some("/override".to_string()),
            ..TurnContextOverride::default()
        };
        ctx.apply_override(&override_ctx);

        assert_eq!(ctx.cwd, Some("/override".to_string()));
        assert_eq!(ctx.approval_policy, Some(ApprovalPolicy::OnRequest));
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
