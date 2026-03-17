use crate::ToolError;
use async_trait::async_trait;
use odyssey_rs_sandbox::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxCellLease,
    SandboxHandle, SandboxProvider,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionAction {
    Allow,
    Deny,
    Ask,
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
    async fn request_tool_approval(&self, tool: &str) -> Result<(), ToolError>;
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
    pub permission_rules: HashMap<String, PermissionAction>,
    pub event_sink: Option<Arc<dyn ToolEventSink>>,
    pub approval_handler: Option<Arc<dyn ToolApprovalHandler>>,
    pub skills: Option<Arc<dyn SkillProvider>>,
}

impl ToolContext {
    pub async fn authorize_tool(&self, name: &str) -> Result<(), ToolError> {
        match self
            .permission_rules
            .get(name)
            .copied()
            .unwrap_or(PermissionAction::Allow)
        {
            PermissionAction::Allow => Ok(()),
            PermissionAction::Deny => Err(ToolError::PermissionDenied(format!(
                "tool {name} is denied"
            ))),
            PermissionAction::Ask => {
                let handler = self.approval_handler.as_ref().ok_or_else(|| {
                    ToolError::PermissionDenied(format!("tool {name} requires approval"))
                })?;
                handler.request_tool_approval(name).await
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
