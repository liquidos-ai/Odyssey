use odyssey_rs_protocol::{ModelSpec, Task, TurnContextOverride};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    pub project_path: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub agent_ref: String,
    #[serde(default)]
    pub model: Option<ModelSpec>,
}

#[derive(Debug, Deserialize)]
pub struct RunRequest {
    pub input: Task,
    #[serde(default)]
    pub turn_context: Option<TurnContextOverride>,
}

#[derive(Debug, Deserialize)]
pub struct PlaceholderRequest {
    pub reference: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub reference: String,
    pub output_path: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportRequest {
    pub archive_path: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveApprovalRequest {
    pub decision: odyssey_rs_protocol::ApprovalDecision,
}

#[derive(Debug, Serialize)]
pub struct TurnAccepted {
    pub session_id: Uuid,
    pub turn_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ApprovalResolution {
    pub resolved: bool,
}
