//! Tool context construction for orchestrator and subagents.

use crate::error::OdysseyCoreError;
use crate::permissions::PermissionEngine;
use log::debug;
use odyssey_rs_protocol::{EventSink, SkillProvider};
use odyssey_rs_sandbox::{
    LocalSandboxProvider, SandboxContext, SandboxEnvPolicy, SandboxFilesystemPolicy, SandboxLimits,
    SandboxNetworkPolicy, SandboxPolicy, SandboxProvider,
};
use odyssey_rs_tools::{
    PermissionChecker, QuestionHandler, ToolContext, ToolOutputPolicy, ToolResultHandler,
    ToolSandbox, TurnServices,
};
use parking_lot::RwLock;
use std::sync::Arc;
use uuid::Uuid;

/// Builds ToolContext instances with sandbox, permissions, and skill wiring.
#[derive(Clone)]
pub(crate) struct ToolContextFactory {
    /// Shared configuration snapshot.
    config: Arc<odyssey_rs_config::OdysseyConfig>,
    /// Optional sandbox provider for tool execution.
    sandbox_provider: Option<Arc<dyn SandboxProvider>>,
    /// Permission engine for tool approvals.
    permission_engine: Arc<PermissionEngine>,
    /// Optional question handler for interactive prompts.
    question_handler: Arc<RwLock<Option<Arc<dyn QuestionHandler>>>>,
    /// Optional skill store for skill metadata.
    skill_store: Option<Arc<dyn SkillProvider>>,
    /// Optional tool event sink for streaming events.
    tool_event_sink: Option<Arc<dyn EventSink>>,
}

#[derive(Clone)]
struct ScopedPermissionChecker {
    engine: Arc<PermissionEngine>,
    event_sink: Option<Arc<dyn EventSink>>,
}

#[async_trait::async_trait]
impl PermissionChecker for ScopedPermissionChecker {
    async fn authorize(
        &self,
        ctx: &odyssey_rs_tools::PermissionContext,
        request: odyssey_rs_protocol::PermissionRequest,
    ) -> Result<odyssey_rs_tools::PermissionOutcome, odyssey_rs_protocol::ToolError> {
        self.engine
            .authorize_with_sink(ctx, request, self.event_sink.clone())
            .await
    }
}

impl ToolContextFactory {
    /// Create a new factory with shared dependencies.
    pub(crate) fn new(
        config: Arc<odyssey_rs_config::OdysseyConfig>,
        sandbox_provider: Option<Arc<dyn SandboxProvider>>,
        permission_engine: Arc<PermissionEngine>,
        question_handler: Arc<RwLock<Option<Arc<dyn QuestionHandler>>>>,
        skill_store: Option<Arc<dyn SkillProvider>>,
        tool_event_sink: Option<Arc<dyn EventSink>>,
    ) -> Self {
        Self {
            config,
            sandbox_provider,
            permission_engine,
            question_handler,
            skill_store,
            tool_event_sink,
        }
    }

    /// Build a per-turn tool context with sandbox and tool result handling.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn build_turn_context(
        &self,
        session_id: Uuid,
        agent_id: &str,
        turn_id: Uuid,
        sandbox_enabled: bool,
        sandbox_mode: odyssey_rs_protocol::SandboxMode,
        tool_result_handler: Option<Arc<dyn ToolResultHandler>>,
        event_sink_override: Option<Arc<dyn EventSink>>,
    ) -> Result<ToolContext, OdysseyCoreError> {
        debug!(
            "building turn tool context (session_id={}, agent_id={}, turn_id={}, sandbox_enabled={})",
            session_id, agent_id, turn_id, sandbox_enabled
        );
        let cwd = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
        let output_policy = Some(output_policy_from_config(&self.config.tools.output_policy));
        let sandbox_policy = sandbox_policy_from_config(&self.config.sandbox);
        let provider = if sandbox_enabled {
            self.sandbox_provider.clone().ok_or_else(|| {
                OdysseyCoreError::Sandbox("sandbox enabled but no provider configured".to_string())
            })?
        } else {
            Arc::new(LocalSandboxProvider::default())
        };
        let handle = provider
            .prepare(&SandboxContext {
                workspace_root: cwd.clone(),
                mode: sandbox_mode,
                policy: sandbox_policy,
            })
            .await
            .map_err(|err| OdysseyCoreError::Sandbox(err.to_string()))?;

        let event_sink = event_sink_override.or_else(|| self.tool_event_sink.clone());
        let permission_checker = ScopedPermissionChecker {
            engine: self.permission_engine.clone(),
            event_sink: event_sink.clone(),
        };
        let services = Arc::new(TurnServices {
            cwd: cwd.clone(),
            workspace_root: cwd,
            output_policy,
            sandbox: Some(ToolSandbox { provider, handle }),
            web: None,
            event_sink,
            skill_provider: self
                .skill_store
                .clone()
                .map(|store| store as Arc<dyn SkillProvider>),
            question_handler: self.question_handler.read().clone(),
            permission_checker: Some(Arc::new(permission_checker)),
            tool_result_handler,
        });

        Ok(ToolContext {
            session_id,
            agent_id: agent_id.to_string(),
            turn_id: Some(turn_id),
            tool_call_id: None,
            tool_name: None,
            services,
        })
    }
}

/// Translate tool output policy config into runtime policy.
fn output_policy_from_config(
    config: &odyssey_rs_config::ToolOutputPolicyConfig,
) -> ToolOutputPolicy {
    ToolOutputPolicy {
        max_string_bytes: config.max_string_bytes,
        max_array_len: config.max_array_len,
        max_object_entries: config.max_object_entries,
        redact_keys: config.redact_keys.clone(),
        redact_values: config.redact_values.clone(),
        replacement: config.replacement.clone(),
    }
}

/// Translate sandbox config into runtime sandbox policy.
fn sandbox_policy_from_config(config: &odyssey_rs_config::SandboxConfig) -> SandboxPolicy {
    SandboxPolicy {
        filesystem: SandboxFilesystemPolicy {
            allow_read: config.filesystem.allow_read.clone(),
            deny_read: config.filesystem.deny_read.clone(),
            allow_write: config.filesystem.allow_write.clone(),
            deny_write: config.filesystem.deny_write.clone(),
            allow_exec: config.filesystem.allow_exec.clone(),
            deny_exec: config.filesystem.deny_exec.clone(),
        },
        env: SandboxEnvPolicy {
            allow: config.env.allow.clone(),
            deny: config.env.deny.clone(),
            set: config.env.set.clone().into_iter().collect(),
        },
        network: SandboxNetworkPolicy {
            allow_domains: config.network.allow_domains.clone(),
            deny_domains: config.network.deny_domains.clone(),
        },
        limits: SandboxLimits {
            cpu_seconds: config.limits.cpu_seconds,
            memory_bytes: config.limits.memory_bytes,
            nofile: config.limits.nofile,
            pids: config.limits.pids,
        },
    }
}
