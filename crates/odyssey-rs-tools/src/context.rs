use crate::ToolError;
use async_trait::async_trait;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxCellLease,
    SandboxHandle, SandboxProvider,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionAction {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionRule {
    pub action: PermissionAction,
    pub matcher: ToolPermissionMatcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionMatcher {
    pub tool: String,
    pub target: Option<String>,
}

impl ToolPermissionMatcher {
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() {
            return Err("tool permission cannot be empty".to_string());
        }

        if let Some(open) = value.find('(') {
            if !value.ends_with(')') {
                return Err("granular tool permission must end with `)`".to_string());
            }
            let tool = value[..open].trim();
            if tool.is_empty() {
                return Err("granular tool permission is missing a tool name".to_string());
            }
            let target = &value[open + 1..value.len() - 1];
            if target.trim().is_empty() {
                return Err("granular tool permission is missing a matcher target".to_string());
            }
            if target.contains('(') || target.contains(')') {
                return Err(
                    "granular tool permission cannot contain nested parentheses".to_string()
                );
            }
            return Ok(Self {
                tool: tool.to_string(),
                target: Some(target.to_string()),
            });
        }

        if value.contains(')') {
            return Err("tool permission has an unmatched closing parenthesis".to_string());
        }

        Ok(Self {
            tool: value.to_string(),
            target: None,
        })
    }

    pub fn display(&self) -> String {
        match &self.target {
            Some(target) => format!("{}({target})", self.tool),
            None => self.tool.clone(),
        }
    }

    fn matches(&self, tool: &str, targets: &[String]) -> bool {
        if self.tool != tool {
            return false;
        }

        match &self.target {
            None => true,
            Some(pattern) => targets
                .iter()
                .any(|target| wildcard_matches(pattern, target)),
        }
    }

    fn specificity(&self) -> u8 {
        if self.target.is_some() { 1 } else { 0 }
    }
}

fn wildcard_matches(pattern: &str, value: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == value;
    }

    let starts_with_star = pattern.starts_with('*');
    let ends_with_star = pattern.ends_with('*');
    let parts = pattern
        .split('*')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return true;
    }

    let mut remainder = value;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 && !starts_with_star {
            if !remainder.starts_with(part) {
                return false;
            }
            remainder = &remainder[part.len()..];
            continue;
        }

        if index == parts.len() - 1 && !ends_with_star {
            return remainder.ends_with(part);
        }

        let Some(found) = remainder.find(part) else {
            return false;
        };
        remainder = &remainder[found + part.len()..];
    }

    true
}

#[derive(Debug, Clone)]
pub enum ToolEvent {
    CommandStarted {
        tool: String,
        exec_id: Uuid,
        command: Vec<String>,
    },
    CommandStdout {
        tool: String,
        exec_id: Uuid,
        line: String,
    },
    CommandStderr {
        tool: String,
        exec_id: Uuid,
        line: String,
    },
    CommandFinished {
        tool: String,
        exec_id: Uuid,
        status: i32,
    },
}

pub trait ToolEventSink: Send + Sync {
    fn emit(&self, event: ToolEvent);
}

pub trait SkillProvider: Send + Sync {
    fn list(&self) -> Vec<SkillEntry>;
    fn load(&self, name: &str) -> Result<String, ToolError>;
}

#[derive(Clone)]
pub struct ToolSandbox {
    pub provider: Arc<dyn SandboxProvider>,
    pub handle: SandboxHandle,
    pub lease: Option<Arc<SandboxCellLease>>,
}

#[async_trait]
pub trait ToolApprovalHandler: Send + Sync {
    async fn request_tool_approval(&self, permission: &str) -> Result<(), ToolError>;
}

#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct ToolContext {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub bundle_root: PathBuf,
    pub working_dir: PathBuf,
    pub sandbox: ToolSandbox,
    pub permission_rules: Vec<ToolPermissionRule>,
    pub event_sink: Option<Arc<dyn ToolEventSink>>,
    pub approval_handler: Option<Arc<dyn ToolApprovalHandler>>,
    pub skills: Option<Arc<dyn SkillProvider>>,
}

impl ToolContext {
    pub async fn authorize_tool(&self, name: &str) -> Result<(), ToolError> {
        self.authorize_tool_with_targets(name, &[]).await
    }

    pub async fn authorize_tool_with_targets(
        &self,
        name: &str,
        targets: &[String],
    ) -> Result<(), ToolError> {
        let matched = self
            .permission_rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.matcher.matches(name, targets))
            .max_by_key(|(index, rule)| {
                (
                    rule.matcher.specificity(),
                    permission_action_precedence(rule.action),
                    *index,
                )
            })
            .map(|(_, rule)| rule);
        let (action, permission) = matched.map_or_else(
            || (PermissionAction::Allow, name.to_string()),
            |rule| (rule.action, rule.matcher.display()),
        );
        match action {
            PermissionAction::Allow => Ok(()),
            PermissionAction::Deny => Err(ToolError::PermissionDenied(format!(
                "tool {permission} is denied"
            ))),
            PermissionAction::Ask => {
                let handler = self.approval_handler.as_ref().ok_or_else(|| {
                    ToolError::PermissionDenied(format!("tool {name} requires approval"))
                })?;
                handler.request_tool_approval(&permission).await
            }
        }
    }

    pub fn resolve_workspace_path(&self, path: &str) -> Result<PathBuf, ToolError> {
        let candidate = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.bundle_root.join(path)
        };
        Ok(candidate)
    }

    pub fn check_read(&self, path: &Path) -> Result<(), ToolError> {
        self.check_access(path, AccessMode::Read)
    }

    pub fn check_write(&self, path: &Path) -> Result<(), ToolError> {
        self.check_access(path, AccessMode::Write)
    }

    pub fn check_execute(&self, path: &Path) -> Result<(), ToolError> {
        self.check_access(path, AccessMode::Execute)
    }

    fn check_access(&self, path: &Path, mode: AccessMode) -> Result<(), ToolError> {
        match self
            .sandbox
            .provider
            .check_access(&self.sandbox.handle, path, mode)
        {
            AccessDecision::Allow => Ok(()),
            AccessDecision::Deny(reason) => Err(ToolError::PermissionDenied(reason)),
        }
    }

    pub async fn run_command(
        &self,
        tool: &str,
        spec: CommandSpec,
    ) -> Result<CommandResult, ToolError> {
        let exec_id = Uuid::new_v4();
        if let Some(sink) = &self.event_sink {
            let mut command = vec![spec.command.to_string_lossy().to_string()];
            command.extend(spec.args.clone());
            sink.emit(ToolEvent::CommandStarted {
                tool: tool.to_string(),
                exec_id,
                command,
            });
        }
        let mut sink = self.event_sink.as_ref().map(|inner| StreamingCommandSink {
            tool: tool.to_string(),
            exec_id,
            inner: inner.clone(),
        });
        let mut null_sink = NullCommandSink;
        let output_sink: &mut dyn CommandOutputSink = match sink.as_mut() {
            Some(sink) => sink,
            None => &mut null_sink,
        };
        let output = self
            .sandbox
            .provider
            .run_command_streaming(&self.sandbox.handle, spec, output_sink)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        if let Some(event_sink) = &self.event_sink {
            event_sink.emit(ToolEvent::CommandFinished {
                tool: tool.to_string(),
                exec_id,
                status: output.status_code.unwrap_or(-1),
            });
        }
        Ok(output)
    }
}

fn permission_action_precedence(action: PermissionAction) -> u8 {
    match action {
        PermissionAction::Allow => 0,
        PermissionAction::Ask => 1,
        PermissionAction::Deny => 2,
    }
}

struct StreamingCommandSink {
    tool: String,
    exec_id: Uuid,
    inner: Arc<dyn ToolEventSink>,
}

struct NullCommandSink;

impl CommandOutputSink for NullCommandSink {
    fn stdout(&mut self, _chunk: &str) {}

    fn stderr(&mut self, _chunk: &str) {}
}

impl CommandOutputSink for StreamingCommandSink {
    fn stdout(&mut self, chunk: &str) {
        self.inner.emit(ToolEvent::CommandStdout {
            tool: self.tool.clone(),
            exec_id: self.exec_id,
            line: chunk.to_string(),
        });
    }

    fn stderr(&mut self, chunk: &str) {
        self.inner.emit(ToolEvent::CommandStderr {
            tool: self.tool.clone(),
            exec_id: self.exec_id,
            line: chunk.to_string(),
        });
    }
}
