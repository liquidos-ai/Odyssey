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

#[cfg(test)]
mod tests {
    use super::{
        ApprovalResolution, BuildRequest, CreateSessionRequest, ExportRequest, ImportRequest,
        PlaceholderRequest, PublishRequest, ResolveApprovalRequest, RunRequest, TurnAccepted,
    };
    use odyssey_rs_protocol::{ApprovalDecision, Task};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn request_models_deserialize_expected_payloads() {
        let task = Task::new("hello");
        let build: BuildRequest = serde_json::from_value(json!({
            "project_path": "/tmp/project"
        }))
        .expect("build request");
        let create: CreateSessionRequest = serde_json::from_value(json!({
            "agent_ref": "demo@0.1.0"
        }))
        .expect("create session request");
        let run: RunRequest = serde_json::from_value(json!({
            "input": serde_json::to_value(&task).expect("serialize task")
        }))
        .expect("run request");
        let placeholder: PlaceholderRequest = serde_json::from_value(json!({
            "reference": "team/demo:0.1.0"
        }))
        .expect("placeholder request");
        let publish: PublishRequest = serde_json::from_value(json!({
            "source": "demo@0.1.0",
            "target": "team/demo:0.1.0"
        }))
        .expect("publish request");
        let export: ExportRequest = serde_json::from_value(json!({
            "reference": "demo@0.1.0",
            "output_path": "/tmp/out"
        }))
        .expect("export request");
        let import: ImportRequest = serde_json::from_value(json!({
            "archive_path": "/tmp/demo.odyssey"
        }))
        .expect("import request");
        let approval: ResolveApprovalRequest = serde_json::from_value(json!({
            "decision": "deny"
        }))
        .expect("approval request");

        assert_eq!(build.project_path, "/tmp/project");
        assert_eq!(create.agent_ref, "demo@0.1.0");
        assert!(create.model.is_none());
        assert!(run.turn_context.is_none());
        assert_eq!(run.input.prompt, "hello");
        assert_eq!(run.input.system_prompt, None);
        assert_eq!(placeholder.reference, "team/demo:0.1.0");
        assert_eq!(publish.source, "demo@0.1.0");
        assert_eq!(publish.target, "team/demo:0.1.0");
        assert_eq!(export.reference, "demo@0.1.0");
        assert_eq!(export.output_path, "/tmp/out");
        assert_eq!(import.archive_path, "/tmp/demo.odyssey");
        assert_eq!(approval.decision, ApprovalDecision::Deny);
    }

    #[test]
    fn response_models_serialize_stable_payload_shape() {
        let turn_accepted = TurnAccepted {
            session_id: Uuid::nil(),
            turn_id: Uuid::from_u128(1),
        };
        let approval = ApprovalResolution { resolved: true };

        assert_eq!(
            serde_json::to_value(turn_accepted).expect("serialize turn response"),
            json!({
                "session_id": "00000000-0000-0000-0000-000000000000",
                "turn_id": "00000000-0000-0000-0000-000000000001"
            })
        );
        assert_eq!(
            serde_json::to_value(approval).expect("serialize approval response"),
            json!({
                "resolved": true
            })
        );
    }
}
