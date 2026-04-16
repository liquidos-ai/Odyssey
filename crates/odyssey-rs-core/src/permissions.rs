//! Permission enforcement for tools, paths, and commands.

use crate::error::OdysseyCoreError;
use crate::permission_store::ApprovalStore;
use crate::types::SessionId;
use async_trait::async_trait;
use chrono::Utc;
use globset::Glob;
use log::{debug, info, warn};
use odyssey_rs_config::{PermissionMode, PermissionRule, PermissionsConfig};
use odyssey_rs_protocol::EventSink;
use odyssey_rs_protocol::ToolError;
use odyssey_rs_protocol::{
    ApprovalDecision, EventMsg, EventPayload, PathAccess, PermissionAction, PermissionRequest,
};
use odyssey_rs_tools::{PermissionChecker, PermissionContext, PermissionOutcome};
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;
use uuid::Uuid;

/// Result of a permission hook evaluation.
#[derive(Debug, Clone, Copy)]
pub enum HookDecision {
    /// Allow the request immediately.
    Allow,
    /// Deny the request immediately.
    Deny,
    /// Continue evaluating other hooks or rules.
    Continue,
}

/// Hook interface for custom permission decisions.
#[async_trait]
pub trait PermissionHook: Send + Sync {
    /// Evaluate a permission request within its context.
    async fn evaluate(
        &self,
        ctx: &PermissionContext,
        request: &PermissionRequest,
    ) -> Result<HookDecision, ToolError>;
}

/// Request payload passed to approval handlers.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    /// Unique approval request id.
    pub request_id: Uuid,
    /// Session id that triggered the request.
    pub session_id: SessionId,
    /// Agent id that requested the action.
    pub agent_id: String,
    /// Optional turn id for the request.
    pub turn_id: Option<Uuid>,
    /// Action required (allow/deny/ask).
    pub action: PermissionAction,
    /// Original permission request.
    pub request: PermissionRequest,
}

/// Approval handler interface for interactive permission resolution.
#[async_trait]
pub trait ApprovalHandler: Send + Sync {
    /// Request approval and return a decision.
    async fn request_approval(&self, request: ApprovalRequest) -> ApprovalDecision;
}

/// Pending approval stored while waiting for a decision.
#[derive(Debug)]
struct PendingApproval {
    sender: oneshot::Sender<ApprovalDecision>,
    request: ApprovalRequest,
}

/// Compiled matcher for a permission rule.
#[derive(Debug)]
struct RuleMatcher {
    action: PermissionAction,
    tool: Option<String>,
    path: Option<globset::GlobMatcher>,
    path_raw: Option<String>,
    command: Option<Vec<String>>,
    access: Option<PathAccess>,
}

/// Permission engine implementing approval rules and hooks.
pub struct PermissionEngine {
    rules: Vec<RuleMatcher>,
    default_mode: PermissionMode,
    agent_modes: RwLock<HashMap<String, PermissionMode>>,
    hooks: RwLock<Vec<Arc<dyn PermissionHook>>>,
    approval_store: Mutex<ApprovalStore>,
    pending: Mutex<HashMap<Uuid, PendingApproval>>,
    approval_handler: RwLock<Option<Arc<dyn ApprovalHandler>>>,
    event_sink: RwLock<Option<Arc<dyn EventSink>>>,
}

impl PermissionEngine {
    /// Create a new permission engine from config.
    pub fn new(config: PermissionsConfig) -> Result<Self, OdysseyCoreError> {
        let workspace_root = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
        let approval_store = ApprovalStore::load_default(&workspace_root)?;
        Self::new_with_store(config, approval_store)
    }

    fn new_with_store(
        config: PermissionsConfig,
        approval_store: ApprovalStore,
    ) -> Result<Self, OdysseyCoreError> {
        let rules = compile_rules(config.rules)?;
        Ok(Self {
            rules,
            default_mode: config.mode,
            agent_modes: RwLock::new(HashMap::new()),
            hooks: RwLock::new(Vec::new()),
            approval_store: Mutex::new(approval_store),
            pending: Mutex::new(HashMap::new()),
            approval_handler: RwLock::new(None),
            event_sink: RwLock::new(None),
        })
    }

    /// Attach an event sink for permission events.
    pub fn set_event_sink(&self, sink: Option<Arc<dyn EventSink>>) {
        *self.event_sink.write() = sink;
    }

    fn resolve_event_sink(
        &self,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Option<Arc<dyn EventSink>> {
        event_sink.or_else(|| self.event_sink.read().clone())
    }

    /// Register an approval handler for interactive decisions.
    pub fn set_approval_handler(&self, handler: Option<Arc<dyn ApprovalHandler>>) {
        *self.approval_handler.write() = handler;
    }

    /// Add a permission hook to be evaluated before rules.
    pub fn add_hook(&self, hook: Arc<dyn PermissionHook>) {
        self.hooks.write().push(hook);
    }

    /// Register a per-agent permission mode override.
    pub fn register_agent_mode(&self, agent_id: String, mode: Option<PermissionMode>) {
        let mut agent_modes = self.agent_modes.write();
        if let Some(mode) = mode {
            agent_modes.insert(agent_id, mode);
        } else {
            agent_modes.remove(&agent_id);
        }
    }

    /// Resolve a pending approval by request id.
    pub fn resolve_approval(&self, request_id: Uuid, decision: ApprovalDecision) -> bool {
        if let Some(pending) = self.pending.lock().remove(&request_id) {
            info!(
                "approval resolved (request_id={}, decision={:?})",
                request_id, decision
            );
            let _ = pending.sender.send(decision);
            return true;
        }
        false
    }

    /// List pending approval requests.
    pub fn list_pending_approvals(&self) -> Vec<ApprovalRequest> {
        self.pending
            .lock()
            .values()
            .map(|pending| pending.request.clone())
            .collect()
    }

    /// Determine the permission mode for a given agent.
    fn mode_for_agent(&self, agent_id: &str) -> PermissionMode {
        self.agent_modes
            .read()
            .get(agent_id)
            .copied()
            .unwrap_or(self.default_mode)
    }

    /// Check whether a tool is explicitly allowed by rules.
    fn tool_allowed_by_rules(&self, tool_name: &str) -> bool {
        matches!(
            self.rule_action_for_request(&PermissionRequest::Tool {
                name: tool_name.to_string(),
            }),
            Some(PermissionAction::Allow)
        )
    }

    /// Emit a permission requested event.
    fn emit_permission_requested(
        &self,
        ctx: &PermissionContext,
        request_id: Uuid,
        action: PermissionAction,
        request: PermissionRequest,
        event_sink: Option<Arc<dyn EventSink>>,
    ) {
        let Some(sink) = self.resolve_event_sink(event_sink) else {
            return;
        };
        let Some(turn_id) = ctx.turn_id else {
            return;
        };
        debug!(
            "permission requested (request_id={}, session_id={}, agent_id={}, action={:?})",
            request_id, ctx.session_id, ctx.agent_id, action
        );
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id: ctx.session_id,
            created_at: Utc::now(),
            payload: EventPayload::PermissionRequested {
                turn_id,
                request_id,
                action,
                request,
            },
        };
        sink.emit(event);
    }

    /// Emit an approval resolved event.
    fn emit_approval_resolved(
        &self,
        ctx: &PermissionContext,
        request_id: Uuid,
        decision: ApprovalDecision,
        event_sink: Option<Arc<dyn EventSink>>,
    ) {
        let Some(sink) = self.resolve_event_sink(event_sink) else {
            return;
        };
        let Some(turn_id) = ctx.turn_id else {
            return;
        };
        debug!(
            "approval event resolved (request_id={}, session_id={}, decision={:?})",
            request_id, ctx.session_id, decision
        );
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id: ctx.session_id,
            created_at: Utc::now(),
            payload: EventPayload::ApprovalResolved {
                turn_id,
                request_id,
                decision,
            },
        };
        sink.emit(event);
    }

    /// Retrieve a cached approval decision for repeated requests.
    fn lookup_cached_approval(&self, request: &PermissionRequest) -> Option<ApprovalDecision> {
        let key = request_key(request);
        self.approval_store.lock().lookup(&key)
    }

    /// Cache approval decisions that allow repeated execution.
    fn cache_approval(&self, request: &PermissionRequest, decision: ApprovalDecision) {
        if decision != ApprovalDecision::AllowAlways {
            return;
        }
        let key = request_key(request);
        if let Err(err) = self.approval_store.lock().record_allow_always(key) {
            warn!("failed to persist approval: {err}");
        }
    }

    /// Apply permission hooks and return a decision if any hook resolves it.
    async fn apply_hook_decisions(
        &self,
        ctx: &PermissionContext,
        request: &PermissionRequest,
    ) -> Result<Option<PermissionOutcome>, ToolError> {
        let hooks = self.hooks.read().clone();
        for hook in hooks {
            match hook.evaluate(ctx, request).await? {
                HookDecision::Allow => {
                    return Ok(Some(PermissionOutcome {
                        allowed: true,
                        reason: None,
                    }));
                }
                HookDecision::Deny => {
                    return Ok(Some(PermissionOutcome {
                        allowed: false,
                        reason: Some("denied by hook".to_string()),
                    }));
                }
                HookDecision::Continue => (),
            }
        }
        Ok(None)
    }

    /// Determine the action that matches a request based on rules.
    fn rule_action_for_request(&self, request: &PermissionRequest) -> Option<PermissionAction> {
        for action in [
            PermissionAction::Deny,
            PermissionAction::Allow,
            PermissionAction::Ask,
        ] {
            for rule in &self.rules {
                if rule.action == action && rule_matches(rule, request) {
                    return Some(action);
                }
            }
        }
        None
    }

    /// Ask the approval handler or wait for a manual decision.
    async fn ask_for_approval(
        &self,
        ctx: &PermissionContext,
        request: PermissionRequest,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Result<PermissionOutcome, ToolError> {
        let resolved_sink = self.resolve_event_sink(event_sink.clone());
        if let Some(decision) = self.lookup_cached_approval(&request) {
            return Ok(outcome_from_decision(decision));
        }

        let request_id = Uuid::new_v4();
        let action = PermissionAction::Ask;
        self.emit_permission_requested(
            ctx,
            request_id,
            action,
            request.clone(),
            event_sink.clone(),
        );

        let handler = self.approval_handler.read().clone();
        if let Some(handler) = handler {
            let decision = handler
                .request_approval(ApprovalRequest {
                    request_id,
                    session_id: ctx.session_id,
                    agent_id: ctx.agent_id.clone(),
                    turn_id: ctx.turn_id,
                    action,
                    request: request.clone(),
                })
                .await;
            self.cache_approval(&request, decision);
            self.emit_approval_resolved(ctx, request_id, decision, event_sink);
            return Ok(outcome_from_decision(decision));
        }

        if resolved_sink.is_none() {
            if matches!(self.mode_for_agent(&ctx.agent_id), PermissionMode::Default) {
                info!(
                    "permission requested without approval handler or event sink; defaulting to allow"
                );
                return Ok(PermissionOutcome {
                    allowed: true,
                    reason: None,
                });
            }
            warn!("permission requested without approval handler or event sink");
            return Ok(PermissionOutcome {
                allowed: false,
                reason: Some("no approval handler configured".to_string()),
            });
        }

        let (sender, receiver) = oneshot::channel();
        let approval_request = ApprovalRequest {
            request_id,
            session_id: ctx.session_id,
            agent_id: ctx.agent_id.clone(),
            turn_id: ctx.turn_id,
            action,
            request: request.clone(),
        };
        self.pending.lock().insert(
            request_id,
            PendingApproval {
                sender,
                request: approval_request,
            },
        );
        let decision = receiver.await.unwrap_or(ApprovalDecision::Deny);
        self.cache_approval(&request, decision);
        self.emit_approval_resolved(ctx, request_id, decision, event_sink);
        Ok(outcome_from_decision(decision))
    }

    /// Authorize a permission request based on hooks, rules, and mode.
    pub async fn authorize_with_sink(
        &self,
        ctx: &PermissionContext,
        request: PermissionRequest,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Result<PermissionOutcome, ToolError> {
        if let Some(outcome) = self.apply_hook_decisions(ctx, &request).await? {
            return Ok(outcome);
        }

        let action = self.rule_action_for_request(&request);
        if let Some(action) = action {
            return match action {
                PermissionAction::Allow => Ok(PermissionOutcome {
                    allowed: true,
                    reason: None,
                }),
                PermissionAction::Deny => Ok(PermissionOutcome {
                    allowed: false,
                    reason: Some("denied by rule".to_string()),
                }),
                PermissionAction::Ask => self.ask_for_approval(ctx, request, event_sink).await,
            };
        }

        if matches!(
            request,
            PermissionRequest::Path { .. }
                | PermissionRequest::ExternalPath { .. }
                | PermissionRequest::Command { .. }
        ) && let Some(tool_name) = ctx.tool_name.as_deref()
            && self.tool_allowed_by_rules(tool_name)
        {
            return Ok(PermissionOutcome {
                allowed: true,
                reason: None,
            });
        }

        match self.mode_for_agent(&ctx.agent_id) {
            PermissionMode::BypassPermissions => Ok(PermissionOutcome {
                allowed: true,
                reason: None,
            }),
            PermissionMode::Plan => Ok(PermissionOutcome {
                allowed: false,
                reason: Some("plan mode blocks tool execution".to_string()),
            }),
            PermissionMode::AcceptEdits => {
                if accept_edits_allows(&request) {
                    Ok(PermissionOutcome {
                        allowed: true,
                        reason: None,
                    })
                } else {
                    self.ask_for_approval(ctx, request, event_sink).await
                }
            }
            PermissionMode::Default => self.ask_for_approval(ctx, request, event_sink).await,
        }
    }
}

#[async_trait]
impl PermissionChecker for PermissionEngine {
    /// Authorize a permission request based on hooks, rules, and mode.
    async fn authorize(
        &self,
        ctx: &PermissionContext,
        request: PermissionRequest,
    ) -> Result<PermissionOutcome, ToolError> {
        self.authorize_with_sink(ctx, request, None).await
    }
}

/// Compile configured permission rules into matchers.
fn compile_rules(rules: Vec<PermissionRule>) -> Result<Vec<RuleMatcher>, OdysseyCoreError> {
    rules
        .into_iter()
        .map(|rule| {
            let path = match rule.path.as_ref() {
                Some(pattern) => Some(
                    Glob::new(pattern)
                        .map_err(|err| OdysseyCoreError::Parse(err.to_string()))?
                        .compile_matcher(),
                ),
                None => None,
            };
            let access = rule.access;
            Ok(RuleMatcher {
                action: rule.action,
                tool: rule.tool,
                path,
                path_raw: rule.path,
                command: rule.command,
                access,
            })
        })
        .collect()
}

/// Determine whether a rule matches a permission request.
fn rule_matches(rule: &RuleMatcher, request: &PermissionRequest) -> bool {
    let has_filters = rule.tool.is_some()
        || rule.path.is_some()
        || rule.command.is_some()
        || rule.access.is_some();
    if !has_filters {
        return true;
    }
    match request {
        PermissionRequest::Tool { name } => {
            if rule.path.is_some() || rule.command.is_some() || rule.access.is_some() {
                return false;
            }
            rule.tool
                .as_ref()
                .is_none_or(|tool| tool == "*" || tool == name)
        }
        PermissionRequest::Path { path, mode } | PermissionRequest::ExternalPath { path, mode } => {
            if rule.tool.is_some() || rule.command.is_some() {
                return false;
            }
            if let Some(access) = rule.access
                && access != *mode
            {
                return false;
            }
            if let Some(matcher) = rule.path.as_ref() {
                return matcher.is_match(path);
            }
            rule.path_raw.is_none()
        }
        PermissionRequest::Command { argv } => {
            if rule.tool.is_some() || rule.path.is_some() || rule.access.is_some() {
                return false;
            }
            let Some(prefix) = rule.command.as_ref() else {
                return false;
            };
            argv.starts_with(prefix)
        }
    }
}

/// Determine if accept-edits mode allows the request without approval.
fn accept_edits_allows(request: &PermissionRequest) -> bool {
    match request {
        PermissionRequest::Tool { name } => {
            matches!(name.as_str(), "Read" | "Write" | "Edit" | "Glob" | "Grep")
        }
        PermissionRequest::Path { .. } => true,
        PermissionRequest::ExternalPath { .. } => false,
        PermissionRequest::Command { .. } => false,
    }
}

/// Generate a stable cache key for a permission request.
fn request_key(request: &PermissionRequest) -> String {
    match request {
        PermissionRequest::Tool { name } => format!("tool:{name}"),
        PermissionRequest::Path { path, mode } => format!("path:{mode:?}:{path}"),
        PermissionRequest::ExternalPath { path, mode } => format!("external:{mode:?}:{path}"),
        PermissionRequest::Command { argv } => format!("command:{}", argv.join(" ")),
    }
}

/// Convert an approval decision into a permission outcome.
fn outcome_from_decision(decision: ApprovalDecision) -> PermissionOutcome {
    PermissionOutcome {
        allowed: decision != ApprovalDecision::Deny,
        reason: if decision == ApprovalDecision::Deny {
            Some("denied by user".to_string())
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use odyssey_rs_config::{PermissionAction, PermissionMode, PermissionRule, PermissionsConfig};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct StaticApprovalHandler {
        decision: ApprovalDecision,
    }

    #[async_trait]
    impl ApprovalHandler for StaticApprovalHandler {
        async fn request_approval(&self, _request: ApprovalRequest) -> ApprovalDecision {
            self.decision
        }
    }

    fn temp_workspace() -> TempDir {
        TempDir::new().expect("tempdir")
    }

    fn engine_with_store(
        config: PermissionsConfig,
        workspace_root: &Path,
        store_path: PathBuf,
    ) -> PermissionEngine {
        let store = ApprovalStore::load(workspace_root, store_path).expect("approval store");
        PermissionEngine::new_with_store(config, store).expect("engine")
    }

    #[tokio::test]
    async fn ask_rule_blocks_implicit_tool_allow() {
        let workspace = temp_workspace();
        let store_path = workspace.path().join("permission.jsonl");
        let config = PermissionsConfig {
            mode: PermissionMode::Default,
            rules: vec![
                PermissionRule {
                    action: PermissionAction::Allow,
                    tool: Some("Write".to_string()),
                    path: None,
                    command: None,
                    access: None,
                },
                PermissionRule {
                    action: PermissionAction::Ask,
                    tool: None,
                    path: Some("odyssey_test/ask_override.txt".to_string()),
                    command: None,
                    access: Some(PathAccess::Write),
                },
            ],
        };
        let engine = engine_with_store(config, workspace.path(), store_path);
        engine.set_approval_handler(Some(Arc::new(StaticApprovalHandler {
            decision: ApprovalDecision::Deny,
        })));

        let ctx = PermissionContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            tool_name: Some("Write".to_string()),
            turn_id: None,
        };
        let outcome = engine
            .authorize(
                &ctx,
                PermissionRequest::Path {
                    path: "odyssey_test/ask_override.txt".to_string(),
                    mode: PathAccess::Write,
                },
            )
            .await
            .expect("outcome");

        assert_eq!(outcome.allowed, false);
        assert_eq!(outcome.reason.as_deref(), Some("denied by user"));
    }

    #[tokio::test]
    async fn allow_always_persists_across_engines() {
        let workspace = temp_workspace();
        let store_path = workspace.path().join("permission.jsonl");
        let config = PermissionsConfig {
            mode: PermissionMode::Default,
            rules: vec![PermissionRule {
                action: PermissionAction::Ask,
                tool: Some("Read".to_string()),
                path: None,
                command: None,
                access: None,
            }],
        };

        let engine = engine_with_store(config.clone(), workspace.path(), store_path.clone());
        engine.set_approval_handler(Some(Arc::new(StaticApprovalHandler {
            decision: ApprovalDecision::AllowAlways,
        })));
        let ctx = PermissionContext {
            session_id: Uuid::new_v4(),
            agent_id: "agent".to_string(),
            tool_name: None,
            turn_id: None,
        };
        let outcome = engine
            .authorize(
                &ctx,
                PermissionRequest::Tool {
                    name: "Read".to_string(),
                },
            )
            .await
            .expect("outcome");
        assert_eq!(outcome.allowed, true);

        let engine = engine_with_store(config, workspace.path(), store_path);
        engine.set_approval_handler(Some(Arc::new(StaticApprovalHandler {
            decision: ApprovalDecision::Deny,
        })));
        let outcome = engine
            .authorize(
                &ctx,
                PermissionRequest::Tool {
                    name: "Read".to_string(),
                },
            )
            .await
            .expect("outcome");
        assert_eq!(outcome.allowed, true);
        assert_eq!(outcome.reason, None);
    }
}
