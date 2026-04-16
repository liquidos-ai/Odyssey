//! Tool execution context and permission helpers.

use crate::Tool;
use crate::events::EventSink;
use crate::output_policy::ToolOutputPolicy;
use crate::permissions::{PermissionChecker, PermissionContext};
use crate::question::QuestionHandler;
use crate::web::WebProvider;
use async_trait::async_trait;
use chrono::Utc;
use log::{debug, warn};
use odyssey_rs_protocol::{EventMsg, EventPayload, PathAccess, PermissionRequest, ToolCallId};
use odyssey_rs_protocol::{SkillProvider, ToolError};
use odyssey_rs_sandbox::{AccessDecision, AccessMode, SandboxHandle, SandboxProvider};
use serde_json::Value;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

/// Sandbox handle and provider attached to a tool context.
#[derive(Clone)]
pub struct ToolSandbox {
    /// Sandbox provider implementation.
    pub provider: Arc<dyn SandboxProvider>,
    /// Provider-specific sandbox handle.
    pub handle: SandboxHandle,
}

/// Shared service dependencies for a turn (constructed once, shared via Arc).
pub struct TurnServices {
    /// Current working directory.
    pub cwd: PathBuf,
    /// Workspace root directory.
    pub workspace_root: PathBuf,
    /// Output policy applied to tool results.
    pub output_policy: Option<ToolOutputPolicy>,
    /// Sandbox configuration if enabled.
    pub sandbox: Option<ToolSandbox>,
    /// Optional web provider for network tools.
    pub web: Option<Arc<dyn WebProvider>>,
    /// Optional event sink for tool events.
    pub event_sink: Option<Arc<dyn EventSink>>,
    /// Optional skill provider for skill tools.
    pub skill_provider: Option<Arc<dyn SkillProvider>>,
    /// Optional question handler for interactive tools.
    pub question_handler: Option<Arc<dyn QuestionHandler>>,
    /// Optional permission checker for gated actions.
    pub permission_checker: Option<Arc<dyn PermissionChecker>>,
    /// Optional handler for recording tool results.
    pub tool_result_handler: Option<Arc<dyn ToolResultHandler>>,
}

/// Shared context passed to tools during execution.
///
/// Per-invocation identity fields are stored directly.
/// Shared service references live behind an `Arc<TurnServices>` so cloning
/// per tool call is a cheap reference-count bump.
#[derive(Clone)]
pub struct ToolContext {
    /// Session id associated with the tool call.
    pub session_id: Uuid,
    /// Agent id that requested the tool.
    pub agent_id: String,
    /// Optional turn id for the tool call.
    pub turn_id: Option<Uuid>,
    /// Optional tool call id for this invocation.
    pub tool_call_id: Option<Uuid>,
    /// Tool name for the current invocation.
    pub tool_name: Option<String>,
    /// Shared turn-scoped services (cheap Arc clone).
    pub services: Arc<TurnServices>,
}

#[async_trait]
/// Tool result handler interface used to capture tool outputs.
pub trait ToolResultHandler: Send + Sync {
    /// Record a tool invocation result for this context.
    async fn record_tool_result(
        &self,
        ctx: &ToolContext,
        name: &str,
        args: &Value,
        result: &Value,
    ) -> Result<(), ToolError>;
}

impl ToolContext {
    /// Apply the configured output policy to a tool result value.
    pub fn apply_output_policy(&self, value: Value) -> Value {
        match self.services.output_policy.as_ref() {
            Some(policy) => policy.apply(value),
            None => value,
        }
    }

    /// Check sandbox access for a filesystem path.
    pub fn check_access(&self, path: &std::path::Path, mode: AccessMode) -> Result<(), ToolError> {
        let Some(sandbox) = &self.services.sandbox else {
            return Ok(());
        };
        match sandbox.provider.check_access(&sandbox.handle, path, mode) {
            AccessDecision::Allow => Ok(()),
            AccessDecision::Deny(reason) => Err(ToolError::PermissionDenied(reason)),
        }
    }

    /// Build a permission context for this tool invocation.
    pub fn permission_context(&self) -> PermissionContext {
        PermissionContext {
            session_id: self.session_id,
            agent_id: self.agent_id.clone(),
            tool_name: self.tool_name.clone(),
            turn_id: self.turn_id,
        }
    }

    /// Authorize a permission request via the configured checker.
    pub async fn authorize(&self, request: PermissionRequest) -> Result<(), ToolError> {
        let Some(checker) = &self.services.permission_checker else {
            return Ok(());
        };
        debug!(
            "authorizing permission request (session_id={}, agent_id={})",
            self.session_id, self.agent_id
        );
        let context = self.permission_context();
        let outcome = checker.authorize(&context, request).await?;
        if outcome.allowed {
            debug!(
                "authorizing permission suceeded (session_id={}, agent_id={})",
                self.session_id, self.agent_id
            );
            Ok(())
        } else {
            Err(ToolError::PermissionDenied(
                outcome
                    .reason
                    .unwrap_or_else(|| "permission denied".to_string()),
            ))
        }
    }

    /// Authorize a tool invocation by name.
    pub async fn authorize_tool(&self, name: &str) -> Result<(), ToolError> {
        debug!("authorizing tool (name={})", name);
        self.authorize(PermissionRequest::Tool {
            name: name.to_string(),
        })
        .await
    }

    /// Authorize filesystem access for a path.
    pub async fn authorize_path(
        &self,
        path: &std::path::Path,
        mode: PathAccess,
    ) -> Result<(), ToolError> {
        debug!(
            "authorizing path access (mode={:?}, is_workspace={})",
            mode,
            path.starts_with(&self.services.workspace_root)
        );
        let request = if path.starts_with(&self.services.workspace_root) {
            let path_string = path
                .strip_prefix(&self.services.workspace_root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            PermissionRequest::Path {
                path: path_string,
                mode,
            }
        } else {
            let path_string = path.to_string_lossy().to_string();
            PermissionRequest::ExternalPath {
                path: path_string,
                mode,
            }
        };
        self.authorize(request).await
    }

    /// Authorize command execution.
    pub async fn authorize_command(&self, argv: Vec<String>) -> Result<(), ToolError> {
        debug!("authorizing command (argv_len={})", argv.len());
        self.authorize(PermissionRequest::Command { argv }).await
    }

    /// Emit a tool-call started event and return the tool call id.
    pub fn emit_tool_started(&self, name: &str, args: &Value) -> Option<ToolCallId> {
        let turn_id = self.turn_id?;
        let sink = self.services.event_sink.as_ref()?;
        let tool_call_id = Uuid::new_v4();
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id: self.session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallStarted {
                turn_id,
                tool_call_id,
                tool_name: name.to_string(),
                arguments: args.clone(),
            },
        };
        sink.emit(event);
        Some(tool_call_id)
    }

    /// Execute a tool with the full authorization, event, and recording pipeline.
    pub async fn execute_tool(&mut self, tool: &dyn Tool, args: Value) -> Result<Value, ToolError> {
        self.tool_name = Some(tool.name().to_string());
        self.authorize_tool(tool.name()).await?;
        let tool_call_id = self.emit_tool_started(tool.name(), &args);
        self.tool_call_id = tool_call_id;

        let handler = self.services.tool_result_handler.clone();
        let record_args = if handler.is_some() {
            Some(args.clone())
        } else {
            None
        };

        match tool.call(self, args).await {
            Ok(result) => {
                if let (Some(handler), Some(record_args)) = (handler, record_args)
                    && let Err(err) = handler
                        .record_tool_result(self, tool.name(), &record_args, &result)
                        .await
                {
                    warn!(
                        "tool result handler failed (tool_name={}, session_id={}): {}",
                        tool.name(),
                        self.session_id,
                        err
                    );
                }
                let output = self.apply_output_policy(result);
                self.emit_tool_finished(tool_call_id, output.clone(), true);
                Ok(output)
            }
            Err(err) => {
                self.emit_tool_finished(tool_call_id, json!({ "error": err.to_string() }), false);
                Err(err)
            }
        }
    }

    /// Emit a tool-call finished event.
    pub fn emit_tool_finished(
        &self,
        tool_call_id: Option<ToolCallId>,
        result: Value,
        success: bool,
    ) {
        let Some(turn_id) = self.turn_id else {
            return;
        };
        let Some(sink) = self.services.event_sink.as_ref() else {
            return;
        };
        let tool_call_id = tool_call_id.unwrap_or_else(Uuid::new_v4);
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id: self.session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallFinished {
                turn_id,
                tool_call_id,
                result,
                success,
            },
        };
        sink.emit(event);
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("turn_id", &self.turn_id)
            .field("tool_call_id", &self.tool_call_id)
            .field("tool_name", &self.tool_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolContext, ToolResultHandler, TurnServices};
    use crate::Tool;
    use crate::output_policy::ToolOutputPolicy;
    use crate::permissions::{PermissionChecker, PermissionContext, PermissionOutcome};
    use async_trait::async_trait;
    use odyssey_rs_protocol::{EventMsg, PathAccess, PermissionRequest, ToolError};
    use odyssey_rs_sandbox::{AccessMode, LocalSandboxProvider, SandboxContext, SandboxProvider};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    struct StaticPermission {
        allowed: bool,
    }

    #[async_trait]
    impl PermissionChecker for StaticPermission {
        async fn authorize(
            &self,
            _ctx: &PermissionContext,
            _request: PermissionRequest,
        ) -> Result<PermissionOutcome, ToolError> {
            Ok(PermissionOutcome {
                allowed: self.allowed,
                reason: if self.allowed {
                    None
                } else {
                    Some("blocked".to_string())
                },
            })
        }
    }

    struct NullResultHandler;

    #[async_trait]
    impl ToolResultHandler for NullResultHandler {
        async fn record_tool_result(
            &self,
            _ctx: &ToolContext,
            _name: &str,
            _args: &serde_json::Value,
            _result: &serde_json::Value,
        ) -> Result<(), ToolError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: parking_lot::Mutex<VecDeque<EventMsg>>,
    }

    impl odyssey_rs_protocol::EventSink for RecordingSink {
        fn emit(&self, event: EventMsg) {
            self.events.lock().push_back(event);
        }
    }

    fn base_services(root: PathBuf) -> TurnServices {
        TurnServices {
            cwd: root.clone(),
            workspace_root: root,
            output_policy: None,
            sandbox: None,
            web: None,
            event_sink: None,
            skill_provider: None,
            question_handler: None,
            permission_checker: None,
            tool_result_handler: Some(Arc::new(NullResultHandler)),
        }
    }

    #[derive(Debug)]
    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "Dummy"
        }

        fn description(&self) -> &str {
            "dummy tool"
        }

        fn args_schema(&self) -> serde_json::Value {
            json!({})
        }

        async fn call(
            &self,
            _ctx: &ToolContext,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(json!({ "ok": true }))
        }
    }

    #[derive(Debug)]
    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "Failing"
        }

        fn description(&self) -> &str {
            "fails"
        }

        fn args_schema(&self) -> serde_json::Value {
            json!({})
        }

        async fn call(
            &self,
            _ctx: &ToolContext,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Err(ToolError::ExecutionFailed("boom".to_string()))
        }
    }

    #[test]
    fn apply_output_policy_redacts() {
        let temp = tempdir().expect("tempdir");
        let mut services = base_services(temp.path().to_path_buf());
        services.output_policy = Some(ToolOutputPolicy {
            max_string_bytes: 4,
            max_array_len: 8,
            max_object_entries: 8,
            redact_keys: vec!["secret".to_string()],
            redact_values: Vec::new(),
            replacement: "[X]".to_string(),
        });
        let ctx = ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };
        let output = ctx.apply_output_policy(json!({ "secret": "value" }));
        assert_eq!(output, json!({ "secret": "[X]" }));
    }

    #[tokio::test]
    async fn authorize_respects_permission_checker() {
        let temp = tempdir().expect("tempdir");
        let mut services = base_services(temp.path().to_path_buf());
        services.permission_checker = Some(Arc::new(StaticPermission { allowed: false }));

        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };
        let err = ctx
            .authorize(PermissionRequest::Tool {
                name: "Read".to_string(),
            })
            .await
            .expect_err("permission denied");
        match err {
            ToolError::PermissionDenied(message) => assert_eq!(message, "blocked"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_access_denies_outside_workspace() {
        let temp = tempdir().expect("tempdir");
        let provider = LocalSandboxProvider::new();
        let ctx = SandboxContext {
            workspace_root: temp.path().to_path_buf(),
            mode: odyssey_rs_protocol::SandboxMode::WorkspaceWrite,
            policy: odyssey_rs_sandbox::SandboxPolicy::default(),
        };
        let handle = provider.prepare(&ctx).await.expect("prepare");

        let mut services = base_services(temp.path().to_path_buf());
        services.sandbox = Some(super::ToolSandbox {
            provider: Arc::new(provider),
            handle,
        });
        let ctx = ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };
        let outside = tempdir().expect("outside");
        let err = ctx
            .check_access(outside.path(), AccessMode::Read)
            .expect_err("denied");
        match err {
            ToolError::PermissionDenied(message) => assert!(!message.trim().is_empty()),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn authorize_path_allows_without_checker() {
        let temp = tempdir().expect("tempdir");
        let ctx = ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(base_services(temp.path().to_path_buf())),
        };
        ctx.authorize_path(temp.path(), PathAccess::Read)
            .await
            .expect("ok");
    }

    #[tokio::test]
    async fn emit_tool_events_and_execute() {
        let temp = tempdir().expect("tempdir");
        let sink = Arc::new(RecordingSink::default());
        let mut services = base_services(temp.path().to_path_buf());
        services.permission_checker = Some(Arc::new(StaticPermission { allowed: true }));
        services.event_sink = Some(sink.clone());

        let mut ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: Some(Uuid::new_v4()),
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };

        let tool = DummyTool;
        let result = ctx.execute_tool(&tool, json!({})).await.expect("execute");
        assert_eq!(result["ok"], true);

        let events = sink.events.lock();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn authorize_command_allows() {
        let temp = tempdir().expect("tempdir");
        let mut services = base_services(temp.path().to_path_buf());
        services.permission_checker = Some(Arc::new(StaticPermission { allowed: true }));
        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };
        ctx.authorize_command(vec!["echo".to_string(), "ok".to_string()])
            .await
            .expect("authorized");
    }

    #[tokio::test]
    async fn execute_tool_emits_failure_event() {
        let temp = tempdir().expect("tempdir");
        let sink = Arc::new(RecordingSink::default());
        let mut services = base_services(temp.path().to_path_buf());
        services.permission_checker = Some(Arc::new(StaticPermission { allowed: true }));
        services.event_sink = Some(sink.clone());

        let mut ctx = ToolContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            turn_id: Some(Uuid::new_v4()),
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(services),
        };

        let tool = FailingTool;
        let err = ctx
            .execute_tool(&tool, json!({}))
            .await
            .expect_err("failed");
        match err {
            ToolError::ExecutionFailed(message) => assert_eq!(message, "boom".to_string()),
            other => panic!("unexpected error: {other:?}"),
        }

        let events = sink.events.lock();
        assert_eq!(events.len(), 2);
    }
}
