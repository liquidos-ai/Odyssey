use odyssey_rs_tools::{ToolContext, TurnServices};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub fn base_tool_context() -> ToolContext {
    ToolContext {
        session_id: Uuid::nil(),
        agent_id: "agent".to_string(),
        turn_id: None,
        tool_call_id: None,
        tool_name: None,
        services: Arc::new(TurnServices {
            cwd: PathBuf::from("."),
            workspace_root: PathBuf::from("."),
            output_policy: None,
            sandbox: None,
            web: None,
            event_sink: None,
            skill_provider: None,
            question_handler: None,
            permission_checker: None,
            tool_result_handler: None,
        }),
    }
}
